# Выпуск портативного лаунчера

## Однократно: игровой патч 1.03.3

Создать release с постоянным тегом `game-patches` и загрузить в него файл
`nightreign-patch-1.03.3.zip`. Ожидаемая SHA-256:

`1A35C1987EF23E38DAF5DB956824CD6A59B051B240CB7846BA9ECD981F5E7B98`

Файл не нужно коммитить в репозиторий и не нужно повторно прикладывать к каждому
релизу лаунчера.

## Каждый выпуск

1. Обновить `version.txt` и файлы мода, затем закоммитить и отправить **весь репозиторий**.
2. Создать и отправить тег лаунчера:

   ```powershell
   git tag launcher-v1.0.0
   git push origin main --tags
   ```

3. GitHub Actions соберёт `SecularNightreign.exe`, посчитает SHA-256 и добавит
   оба файла в GitHub Release. Пользователям нужен только `.exe` из Releases.

Ручная локальная сборка: `powershell -ExecutionPolicy Bypass -File build-release.ps1`.
