# ADR-002: Яндекс.Диск как отдельный опциональный ingress

- Дата: 2026-08-21
- Статус: accepted
- Заменяет: —

## Контекст

Local-first Dictation и импорт локального media обязаны работать offline. Публичная ссылка Яндекс.Диска требует передачи URL Яндексу и скачивания пользовательского содержимого, но не требует cloud ASR. Если смешать этот путь с core, обещание offline станет ложным, а WebView/network boundary — слишком широким.

## Решение

Yandex import — отдельная capability, выключенная по умолчанию и отзывная в settings. V1 принимает только HTTPS public link одного скачиваемого файла через официальный API: без OAuth, private resources, folders, passwords и обхода no-download. Каждая загрузка запускается явной кнопкой и видима как network stage. Применяются строгая URL normalization, проверка каждого bounded redirect/DNS address, запрет private/link-local, двойной size cap и durable `.part → final`.

Исходная public URL хранится только для resume до durable PCM; temporary direct href не является permanent data. После durable PCM URL/download metadata и downloaded container удаляются. Распознавание остаётся локальным.

Официальная документация Yandex Cloud показывает anonymous flow `GET https://cloud-api.yandex.net/v1/disk/public/resources/download?public_key=...` → `href` для public link: [Connecting to Yandex Disk](https://yandex.cloud/en/docs/datasphere/operations/data/connect-to-ya-disk). При этом общие [условия API Яндекс.Диска](https://yandex.ru/legal/disk_api/ru/) описывают регистрацию/OAuth для доступа к сервису, а условия могут меняться. Поэтому M2 implementation имеет отдельный go/no-go: повторно проверить применимость условий к anonymous public-resource endpoint и не добавлять OAuth как скрытый обход. Если подтверждения нет, Yandex ingress не выпускается в v1, а local-file Notetaker остаётся работоспособным.

## Последствия

- Offline deny-all gate для Dictation/local import сохраняется без исключений.
- Yandex availability/expiry/rate limits становятся recoverable ingress errors, а не ASR errors.
- Security tests обязаны покрывать SSRF/DNS rebinding/redirect, size mismatch, Range restart, expiry и capability revoke.
- API/terms review на дату M2 release является blocker; продукт не обходит требования регистрации и не расширяет scope OAuth.
- UI и product copy не могут говорить «данные никогда не покидают компьютер» без разделения local/Yandex paths.

## Затронутые документы

- `context/architecture/01-обзор/границы.md`
- `context/architecture/02-ядро/способности/импортировать-запись.md`
- `context/architecture/02-ядро/права-доступа.md`
- `context/architecture/03-данные/правила-нерушимые.md`
- `context/architecture/05-стек/технологии.md`
- `context/architecture/07-нефункциональные/безопасность.md`

Текущее состояние: [import capability](../../02-ядро/способности/импортировать-запись.md) · [security boundary](../../07-нефункциональные/безопасность.md#опциональный-yandex-ingress) · [этап M2](../../08-дорожная-карта/roadmap.md#m2--расширение-после-доказанного-mvp)
