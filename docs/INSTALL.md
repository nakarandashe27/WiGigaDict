# Установка WiGigaDict на Windows

Целевая платформа: Windows 10 22H2 x64 и Windows 11 x64. Это personal alpha: полная матрица чистой установки и совместимости Windows 10/11 ещё не завершена. Приложение устанавливается для текущего пользователя и не требует прав администратора.

## 1. Обычная установка из GitHub Releases

1. Откройте <https://github.com/nakarandashe27/WiGigaDict/releases>.
2. Выберите нужный релиз. Версии до `1.0` помечаются как prerelease.
3. Скачайте `WiGigaDict_<версия>_x64-setup.exe` и `SHA256SUMS.txt` из одного релиза.
4. В PowerShell перейдите в папку загрузок и проверьте файл:

```powershell
$installer = Get-ChildItem .\WiGigaDict_*_x64-setup.exe | Select-Object -First 1
Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

Хэш в двух выводах должен совпасть без учёта регистра. После этого запустите `.exe` обычным двойным щелчком.

Установщик пока не подписан Authenticode. SmartScreen может показать предупреждение о неизвестном издателе. Разворачивайте подробности только после сверки имени релиза, адреса репозитория и SHA-256; не отключайте системную защиту глобально.

## 2. Установка через PowerShell

Репозиторий содержит скрипт, который получает последний опубликованный релиз (включая prerelease), скачивает installer и `SHA256SUMS.txt`, обязательно сверяет SHA-256 и только затем запускает установку.

После клонирования репозитория:

```powershell
pwsh -NoProfile -File .\scripts\install-release.ps1
```

Тихая установка:

```powershell
pwsh -NoProfile -File .\scripts\install-release.ps1 -Silent
```

Скачать и проверить, но не запускать:

```powershell
pwsh -NoProfile -File .\scripts\install-release.ps1 -DownloadOnly
```

Конкретная версия и каталог загрузки:

```powershell
pwsh -NoProfile -File .\scripts\install-release.ps1 `
  -Version v0.0.4 `
  -DestinationDirectory "$env:USERPROFILE\Downloads"
```

Скрипт не отключает SmartScreen, не запрашивает elevation и не удаляет предыдущие пользовательские данные.

## 3. Сборка из архива исходников

GitHub автоматически прикладывает к каждому релизу `Source code (zip)` и `Source code (tar.gz)`. Это исходники, а не portable-приложение.

1. Скачайте и распакуйте ZIP.
2. Установите зависимости из раздела «Требования для сборки».
3. Откройте Developer PowerShell for VS 2022 в корне исходников.
4. Выполните:

```powershell
& .\scripts\install-vulkan-sdk.ps1
& .\scripts\build.ps1
```

Готовый installer появится в `target\release\bundle\nsis`.

## 4. Сборка из Git

```powershell
git clone https://github.com/nakarandashe27/WiGigaDict.git
Set-Location .\WiGigaDict
& .\scripts\install-vulkan-sdk.ps1
& .\scripts\build.ps1
```

Для запуска development-версии используйте:

```powershell
pwsh -NoProfile -File .\scripts\dev.ps1
```

### Требования для сборки

- Git и PowerShell 7;
- Visual Studio Build Tools 2022 с MSVC v143, Windows 11 SDK 22621 и C++ CMake tools (Ninja);
- Rust MSVC toolchain (точная версия закреплена в `rust-toolchain.toml`);
- Node.js 24.16.0 и npm 11.9.0;
- доступ к сети для npm-пакетов, Rust crates и Vulkan SDK; модели скачиваются отдельно после установки приложения.

Запускайте обе команды в одной PowerShell 7-сессии: `install-vulkan-sdk.ps1` устанавливает закреплённый SDK и задаёт `VULKAN_SDK` для текущего процесса. Загрузка SDK занимает около 288 МБ, установка — около 1,7 ГБ; helper использует автоматическое принятие лицензий SDK. Ознакомьтесь с условиями SDK до запуска helper. Если SDK уже установлен, можно вместо helper задать `$env:VULKAN_SDK = 'C:\VulkanSDK\1.4.357.0'`.

`build.ps1` собирает frontend, Rust desktop, ASR sidecar и GPU worker, затем создаёт NSIS installer. Сборка worker использует короткую локальную папку `C:\wgd`: нужны права записи в неё и место под build-кэш. Веса моделей в installer не входят.

## Первый запуск

1. Откройте раздел **Модели**.
2. Для первого знакомства выберите `Whisper base`; для измеренного GPU-профиля — `Whisper large-v3-turbo Q5` при наличии Vulkan GPU.
3. Дождитесь скачивания и проверки модели.
4. В **Настройках** выберите микрофон, горячую клавишу и каталог локального архива.
5. Поставьте курсор в поле ввода, удерживайте горячую клавишу, произнесите фразу и отпустите её.

Если вставка не подтверждена, откройте **История**: там можно скопировать/восстановить результат или удалить его вручную.

## Где лежат данные

- пользовательский архив: `%USERPROFILE%\Documents\WiGigaDict` по умолчанию;
- служебная база, модели и recovery: `%LOCALAPPDATA%\WiGigaDict`;
- приложение: `%LOCALAPPDATA%\WiGigaDict` при стандартной per-user установке.

Архив меняется в **Настройки → Локальный архив**. Разрешены только локальные абсолютные пути. Сетевые каталоги не принимаются, потому что архив является частью гарантии локальной сохранности.

## Обновление и удаление

Для обновления скачайте новый installer и запустите его поверх установленной версии. Перед обновлением завершите активную запись.

Удалить программу можно через **Параметры Windows → Приложения**. Пользовательский архив и внутренние данные не следует удалять автоматически. Если нужна полная очистка, сначала сделайте резервную копию, затем вручную удалите выбранную папку архива и `%LOCALAPPDATA%\WiGigaDict`.

## Установка с помощью AI-агента

Используйте vendor-neutral runbook: [AGENT_INSTALL.md](AGENT_INSTALL.md). Он задаёт проверяемый результат и запрещает агенту обходить SmartScreen, пропускать SHA-256 или удалять пользовательские данные.
