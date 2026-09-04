# ADR-007: Shell владеет native frame overlay и активирует существующий экземпляр

- **Дата:** 2026-08-27
- **Статус:** принято

## Контекст

Три дефекта personal alpha были воспроизведены на реальном Windows 10 22H2 runner.

**1. Приложение исчезало сразу после запуска.** Tauri 2.11.5 создаёт все окна из конфигурации внутри `app::setup()` **до** вызова пользовательского setup-hook (`tauri-2.11.5/src/app.rs:2523-2532`). Любая ошибка внутри hook превращалась в `Err` из `build()`, который `lib.rs` разворачивал через `.expect(...)`. Панику в release-сборке не видно: бинарь собран как `windows_subsystem = "windows"`, поэтому `eprintln!` уходит в неподключённый stderr. Fault injection (удалены все кандидаты sidecar) дала точный симптом: главное окно появлялось и процесс умирал через ~700 мс без единого следа. `find_sidecar()` при этом на машине разработчика всегда находил бинарь по fallback-путям от `CARGO_MANIFEST_DIR`, которых нет в установленной раскладке — то есть дефект был структурно невидим в dev-запуске.

Второй запуск завершался ещё тише: `SingleInstanceGuard::acquire` возвращал `AlreadyRunning`, а `run_windows()` делал `return` до создания окон. Пользователь получал «двойной клик — ничего не произошло».

**2. HUD выглядел прямоугольным окном с посторонним крестиком.** `tao` при `decorations: false` **не снимает** `WS_CAPTION | WS_SYSMENU` с реального окна — они убираются только в `to_adjusted_window_styles()` для `AdjustWindowRectEx` (`tao-0.35.3/src/platform_impl/windows/window_state.rs:244,308`). Живое overlay-окно имело стиль `0x14CB0000` (`WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`). Отсюда прямоугольная caption-поверхность вокруг pill и системная кнопка закрытия справа сверху — тот самый «маленький крестик». Из того же рассогласования следовал накопительный дрейф размера: каждый `WM_DPICHANGED` при переносе overlay между мониторами 100%/150% пересчитывал размер через caption-less adjusted styles и терял caption+frame. Измерено: 298x77 → 252x54 → 206x32 физических пикселей, при этом viewport WebView деградировал до 107x6 CSS px и pill обрезался.

**3. «Release»-бинарь от прямого `cargo build --release` грузил dev-сервер вместо встроенного frontend.** Tauri решает dev/production не профилем сборки, а cargo-фичей `custom-protocol` (`tauri-2.11.5/build.rs:256`: `let dev = !has_feature("custom-protocol")`). У `wigigadict-desktop` секции `[features]` не было вовсе, поэтому `cargo build -p wigigadict-desktop --release --locked` давал оптимизированный бинарь в dev-режиме: WebView открывал `build.devUrl` (`http://127.0.0.1:1420`). При работающем Vite такой бинарь выглядел полностью исправным; без Vite главное окно показывало чёрную страницу/ERR_CONNECTION_REFUSED. Доказано CDP: у запущенного release-бинаря обе страницы имели `url: http://127.0.0.1:1420/`.

## Решение

0. **`custom-protocol` включён фичей по умолчанию** (`default = ["custom-protocol"]` → `tauri/custom-protocol`). Теперь любой способ сборки release — прямой `cargo build --release`, CI, `tauri build` — встраивает production frontend (`http://tauri.localhost/`). Dev-режим не пострадал: Tauri CLI запускает dev как `cargo run --no-default-features` (доказано логом `tauri:dev`), поэтому `scripts/dev.ps1` и HMR работают как раньше.
1. **Shell явно владеет native frame overlay.** `overlay::apply_overlay_frame` снимает `WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`, ставит `WS_POPUP`, убирает edge-стили и подтверждает изменение через `SWP_FRAMECHANGED`. `DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED` убирает прямоугольную системную тень. `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` сохраняются без изменений — контракт no-focus из ADR-004 не ослаблен.
2. **Размер и позиция overlay всегда выводятся из DPI целевого монитора**, а не переносятся из текущего оконного прямоугольника: `overlay_placement()` считает физический размер из логических констант 176x56 и `GetDpiForMonitor`. Дрейф стал структурно невозможен, а 100%/125%/150% и несколько мониторов дают один и тот же логический HUD.
3. **Одно постоянное overlay-окно с одним инвариантным размером на все фазы.** Pill 148x34 CSS одинаков для recording и processing; native resize/recreate на фазу и на waveform-событие отсутствует.
4. **Ошибка запуска больше не убивает процесс молча.** `lib.rs::log_startup` дописывает content-free строку в `%LOCALAPPDATA%\WiGigaDict\logs\startup.log`, а `build()` обрабатывается явным `match` вместо `.expect`. Отсутствующий sidecar переводит `SidecarRuntime` в degraded-состояние (`asr_sidecar_missing`) вместо провала setup: shell остаётся доступен и честно сообщает, что распознавание недоступно.
5. **Повторный запуск показывает работающий экземпляр.** Второй процесс отправляет зарегистрированное broadcast-сообщение `WiGigaDict.ActivateMainWindow`; subclass главного окна обрабатывает его и вызывает существующий `show_main_section`, после чего завершается. Kernel-mutex остаётся единственным арбитром единственности и по-прежнему берётся до writer-capable setup.

## Последствия

Tray-режим остаётся допустимым только после явного действия пользователя: `CloseRequested` по-прежнему скрывает окно, но теперь его можно вернуть повторным запуском, а не только через tray. Диагностика запуска перестала зависеть от подключённого stderr.

Мы принимаем, что shell правит стили окна за спиной `tao`. Это безопасно, пока `decorations`, `resizable` и `fullscreen` overlay не меняются в runtime (они зафиксированы в конфигурации), и пока frame применяется повторно при каждом показе — так и сделано. При обновлении `tao`/`tauri` наблюдаемый стиль overlay нужно перепроверить: тесты `overlay_frame_removes_every_caption_and_system_button_style` и `overlay_extended_style_keeps_the_hud_unfocusable_and_edgeless` фиксируют ожидание, но проверяют чистую функцию, а не живое окно.

Расхождение dev/installed в `find_sidecar()` остаётся открытым: fallback-пути от `CARGO_MANIFEST_DIR` продолжают маскировать отсутствие sidecar на машине разработчика. Теперь это не фатально, но packaging-проверка Step 17 обязана подтвердить реальную установленную раскладку.
