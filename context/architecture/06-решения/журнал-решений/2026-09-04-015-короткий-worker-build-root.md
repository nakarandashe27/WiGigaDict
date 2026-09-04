# ADR-015: короткий build root для Vulkan worker

Дата: 2026-09-04

Статус: принято

## Контекст

`whisper-rs-sys` собирает `vulkan-shaders-gen` вложенным CMake `ExternalProject`. Его configure через второй Visual Studio generator может завершиться с `No CMAKE_C_COMPILER could be found`, хотя родительский CMake и MSBuild видят MSVC. Первоначально дефект связывался только с пробелом в пути и staging включался условно. Публичная clean-checkout сборка и отдельный пустой cache воспроизвели тот же отказ без пробелов. Чистая сборка обоих уровней через Ninja с уже инициализированным MSVC прошла. Контрольный прогон также показал, что длинный staging root ломает вложенную проверку компилятора из-за `CMAKE_OBJECT_PATH_MAX`; поэтому коротким должно быть полное имя корня, а не только путь без пробелов.

Кроме того, desktop компилировался раньше worker. Локальная сборка проходила только при наличии ignored worker от предыдущего запуска, а чистый checkout корректно отклонял отсутствующий `externalBin`.

## Решение

- Всегда копировать автономный worker crate в короткий `C:\wgd-worker` перед сборкой. Скрипт отклоняет staging roots длиннее 20 символов; новое имя не смешивает cache с прежним Visual Studio generator.
- На время worker build устанавливать `CMAKE_GENERATOR=Ninja`, находить `ninja.exe` в `PATH` или в CMake tools активной Visual Studio и восстанавливать предыдущее process environment после Cargo.
- Сохранять стабильный staging root для повторного использования Cargo/CMake cache.
- Собирать и размещать sidecar и worker до первой компиляции Tauri desktop.
- В self-contained quality gate готовить bundle inputs до clippy и тестов, чтобы чистый checkout не зависел от ignored артефактов.

## Последствия

Локальная сборка, GitHub Actions и coding-agent используют один и тот же короткий путь и CMake generator независимо от расположения checkout. Параллельные сборки на одной машине должны передавать разные значения `-StagingRoot`.
