# ADR-005: Windows 11 matrix отложена без заявления совместимости

- **Дата:** 2026-08-21
- **Статус:** принято

## Контекст

M0 / Step 3 изначально требовал одну Win32 golden matrix на Windows 10 и Windows 11. Текущий воспроизводимый runner — Windows 10 22H2; локальная Windows 11 VM недоступна, а Hyper-V inventory недоступен даже после разрешённой read-only проверки. Пользователь принял решение не блокировать следующий M0 research-step отсутствующей Windows 11 средой.

## Решение

1. M0 / Step 3 закрывает Win32 risk retirement только на Windows 10 22H2, включая standard controls, реальное Tauri/WRY WebView2 window, VS Code, terminal/Claude Code и browser.
2. Windows 11 не считается протестированной или поддерживаемой этим решением. Матрица Windows 11 вынесена в BL-047 и обязательна до любого заявления совместимости с Windows 11 или публичного Windows release, который включает Windows 11.
3. Fail-closed evidence contract не ослабляется: `SendInput` return count остаётся `transport_only`; при отсутствии target acknowledgement или точного сертифицированного правила результат — `uncertain`, автоматический fallback запрещён после возможной доставки.
4. Windows 11 runbook и harness сохраняются, чтобы выполнить отложенную матрицу без изменения production-кода.

## Последствия

Следующий M0 research-step можно начинать после полной Windows 10 matrix и остальных критериев Step 3. До BL-047 документация, installer и UI не должны обещать Windows 11 compatibility. Если Windows 11 выявит отличие Win32 поведения, решение возвращается в ADR и compatibility registry, а не маскируется общим success.
