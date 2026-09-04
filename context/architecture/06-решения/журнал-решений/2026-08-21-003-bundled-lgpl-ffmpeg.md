# ADR-003: Bundled воспроизводимая LGPL-сборка FFmpeg

- Дата: 2026-08-21
- Статус: accepted
- Заменяет: —

## Контекст

Notetaker должен читать проверенные audio/video containers без требования к обычному пользователю устанавливать codec/runtime. Native Rust decoders не покрывают необходимую video/container matrix, а системный FFmpeg непредсказуем по версии, лицензии и capabilities. Установленный development build `8.1.1-full_build` содержит `--enable-gpl` и не подходит для целевого bundle.

## Решение

В M2 WiGigaDict поставляет собственные exact-pinned `ffmpeg`/`ffprobe` CLI и DLL из воспроизводимого shared LGPL profile. `--enable-gpl` и `--enable-nonfree` запрещены; dependencies выбираются allowlist. Bundle сопровождается exact corresponding source, SHA-256 manifest, license/notices, configure/build recipe, toolchain versions, dependency/SBOM inventory и `changes.diff`.

Production pin и форматная matrix принимаются только после зелёного build spike; до этого никакой найденный системный binary не копируется в installer.

## Последствия

- Installer M1 не меняется; FFmpeg становится обязательной runtime-зависимостью только Notetaker/M2.
- Размер package растёт на измеренный bundle; formats обещаются только после probe/decode matrix.
- Shared/DLL distribution уменьшает LGPL relinkability risk, но добавляет DLL inventory и supply-chain gate.
- Любая смена FFmpeg tag/config/dependency требует повторного license/build/media regression и обновления manifest/source bundle.

## Затронутые документы

- `context/architecture/02-ядро/способности/импортировать-запись.md`
- `context/architecture/05-стек/технологии.md`
- `context/architecture/05-стек/что-не-выбрали.md`
- `context/architecture/05-стек/исследования/2026-08-21-ffmpeg-lgpl-build-spike.md`
- `context/architecture/07-нефункциональные/безопасность.md`
- `context/architecture/08-дорожная-карта/roadmap.md`

Текущее состояние: [FFmpeg spike](../../05-стек/исследования/2026-08-21-ffmpeg-lgpl-build-spike.md) · [стек](../../05-стек/технологии.md#research-track-notetaker-до-m2) · [research gate R1](../../08-дорожная-карта/roadmap.md#research-track-notetaker-до-m2)
