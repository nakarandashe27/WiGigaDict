# ADR-004: узкий windows-rs shim и evidence-first insertion

- **Дата:** 2026-08-21
- **Статус:** принято для последующей production-реализации; OS scope уточнён ADR-005

## Контекст

WiGigaDict должен удерживать foreground target между первым и вторым toggle-нажатием, показывать overlay без активации и вставлять immutable text без ложного успеха. `SendInput` не доказывает, что target принял текст, а UIPI может заблокировать ввод без надёжного диагностического return value. Clipboard fallback также опасен потерей пользовательских данных или дублем после неоднозначной доставки.

## Решение

1. Оставить React overlay в обычном Tauri 2 window и применять узкий Windows-only `windows-rs` shim с `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` и `SW_SHOWNOACTIVATE`. Fork Tauri/WRY runtime не использовать, пока regression matrix не докажет невозможность этого пути.
2. Hotkey admission хранит физическое состояние: первый отдельный key-down начинает операцию, auto-repeat игнорируется до key-up, key-up только переармирует toggle, второй отдельный key-down завершает ту же операцию. Native low-level hook остаётся fallback; production сначала проверяет выбранный Tauri global-shortcut path тем же contract suite.
3. Target snapshot содержит HWND, thread/process identity, executable identity, window/control class и integrity level. Перед каждым методом snapshot перепроверяется; изменившийся, исчезнувший или более привилегированный target немедленно даёт `uncertain` без UAC helper.
4. Порядок методов: Unicode packet через `SendInput`, representable virtual-key `SendInput`, затем clipboard paste. Следующий метод допустим только если предыдущий гарантированно не принял ни одной input unit и не имел возможного side effect. `transport_only` останавливает лестницу как `uncertain`, чтобы не создать дубль.
5. Clipboard method разрешён только когда прежнее содержимое можно lossless сохранить и восстановить. Busy clipboard, неизвестные/несохраняемые форматы и failure восстановления завершаются `uncertain`; retry не выполняется автоматически.
6. `delivered` разрешён только при `target_ack` либо активном `certified_transport` rule для точного versioned `(process, version, window/control class, method)`. До прохождения матрицы каждого заявленного OS registry правил пуст.
7. Spike остаётся отдельным `publish = false` workspace crate и не подключается к Tauri shell. Production module появится только на roadmap Step 12.

## Evidence и последствия

Windows 10 build 19045 fixture подтвердил low-level down/repeat/up replay, 100 циклов no-activate overlay без смены foreground, Unicode `target_ack`, virtual-key `transport_only → uncertain`, а также fail-closed focus/missing/elevated/partial-input/clipboard branches. Реальное Tauri/WRY окно и Chrome fixture прошли no-focus matrix. VS Code честно завершился `transport_only → uncertain` без fallback; Windows Terminal / Claude Code 2.1.239 показал повреждённые glyphs вместо точного маркера и также остаётся `uncertain` без fallback. Post-run terminal JSON отсутствует, поэтому машинные transport counts для этой ручной строки не заявляются. Windows 11 отложена ADR-005 и не заявлена совместимой.

Выбранный путь исключает false success и автоматические дубли ценой более частого recovery. Clipboard fallback может быть недоступен при сложном clipboard payload; это безопаснее, чем потерять пользовательский clipboard.
