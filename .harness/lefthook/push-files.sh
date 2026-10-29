#!/usr/bin/env bash
# pre-push: файлы, которые этот пуш отправляет, — для глобов джобов.
#
# Свой список вместо встроенного у lefthook. Без upstream (первый пуш ветки,
# заведённой с --no-track) lefthook берёт `git diff --name-only HEAD origin/main`:
# сравнение двух вершин, а не от общего предка. Ветка, отставшая от main,
# получала в список чужие правки, влитые в main после ответвления, и дорогие
# джобы гонялись на пуше одних docs.
#
# Здесь база — то, что на origin уже есть у этой ветки (`@{push}`), а без неё —
# общий предок с origin/main. Сравнение тремя точками: только то, что
# появилось в ветке.
set -uo pipefail

if base=$(git rev-parse --verify --quiet '@{push}'); then
  :
elif base=$(git merge-base HEAD origin/main 2>/dev/null); then
  :
else
  # Не с чем сравнить: лучше лишняя проверка, чем молча пропущенная.
  git ls-files
  exit 0
fi

git diff --name-only "$base"...HEAD --
