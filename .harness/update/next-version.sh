#!/usr/bin/env bash
# Выбор версии харнесса для обновления. Зовётся из
# `.github/workflows/harness-update.yml`; логика вынесена сюда, чтобы её можно
# было проверить без GitHub (`.harness/update/tests/next-version.sh`).
#
# Использование:
#   next-version.sh [--want <тег>] <текущая> < теги
#       Теги источника — по одному в строке, имена (`v0.2.0`) или ссылки
#       (`refs/tags/v0.2.0`). Текущая — `_commit` из `.copier-answers.yml`:
#       тег `vX.Y.Z` или вывод git describe `vX.Y.Z-N-g<хеш>` (проект создан
#       не с тега — его версия считается равной тегу, от которого он вырос).
#       Печатает строки `ключ=значение` для $GITHUB_OUTPUT:
#         current=vX.Y.Z   версия проекта;
#         next=vA.B.C      версия обновления, пусто — обновлять нечего;
#         major=true       A больше X (иначе false).
#       Без --want берётся последний тег `vX.Y.Z` по semver, пре-релизы
#       (`v1.0.0-rc.1`) пропускаются. С --want — этот тег, пре-релиз тоже; он
#       обязан быть среди тегов.
#   next-version.sh --lt <a> <b>
#       Код 0, если версия a меньше b, иначе 1.
#
# Код 2 — ошибка аргументов, нераспознанная версия или --want без такого тега.
set -euo pipefail

die() {
  printf 'next-version.sh: %s\n' "$1" >&2
  exit 2
}

# Разбор версии в глобальные ver_major, ver_minor, ver_patch, ver_pre.
# Код 1, если строка — не версия vX.Y.Z[-пре-релиз].
parse() {
  local re='^v([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9A-Za-z.-]+))?$'
  [[ $1 =~ $re ]] || return 1
  ver_major=$((10#${BASH_REMATCH[1]}))
  ver_minor=$((10#${BASH_REMATCH[2]}))
  ver_patch=$((10#${BASH_REMATCH[3]}))
  ver_pre=${BASH_REMATCH[5]}
}

# Код 0, если версия $1 меньше $2 по semver. Обе уже проверены parse.
# Пре-релиз меньше релиза той же X.Y.Z; два пре-релиза сравниваются как
# версии (`sort -V`) — точного порядка semver для них здесь не нужно.
less() {
  local a_major a_minor a_patch a_pre
  parse "$1"
  a_major=$ver_major a_minor=$ver_minor a_patch=$ver_patch a_pre=$ver_pre
  parse "$2"
  if ((a_major != ver_major)); then
    ((a_major < ver_major)) && return 0 || return 1
  fi
  if ((a_minor != ver_minor)); then
    ((a_minor < ver_minor)) && return 0 || return 1
  fi
  if ((a_patch != ver_patch)); then
    ((a_patch < ver_patch)) && return 0 || return 1
  fi
  if [[ -n $a_pre && -z $ver_pre ]]; then
    return 0
  fi
  if [[ -z $a_pre || $a_pre == "$ver_pre" ]]; then
    return 1
  fi
  [[ $(printf '%s\n%s\n' "$a_pre" "$ver_pre" | sort -V | head -n 1) == "$a_pre" ]]
}

if [[ ${1:-} == --lt ]]; then
  [[ $# -eq 3 ]] || die "--lt ждёт две версии"
  parse "$2" || die "не версия: $2"
  parse "$3" || die "не версия: $3"
  less "$2" "$3"
  exit
fi

want=""
if [[ ${1:-} == --want ]]; then
  [[ -n ${2:-} ]] || die "--want ждёт тег"
  want=$2
  shift 2
fi
[[ $# -eq 1 ]] || die "ждём одну текущую версию (_commit из .copier-answers.yml)"

# Хвост git describe `-N-g<хеш>` — коммиты после тега: версия та же.
current=$1
if [[ $current =~ ^(.+)-[0-9]+-g[0-9a-f]+$ ]]; then
  current=${BASH_REMATCH[1]}
fi
parse "$current" || die "текущая версия не распознана: $1 (ждём vX.Y.Z или vX.Y.Z-N-g<хеш>)"
current_major=$ver_major

next=""
while IFS= read -r tag; do
  tag=${tag#refs/tags/}
  [[ -n $tag ]] || continue
  parse "$tag" || continue
  if [[ -n $want ]]; then
    [[ $tag == "$want" ]] && next=$tag
    continue
  fi
  [[ -z $ver_pre ]] || continue
  if [[ -z $next ]] || less "$next" "$tag"; then
    next=$tag
  fi
done

if [[ -n $want && -z $next ]]; then
  die "тега $want нет среди тегов источника"
fi

major=false
if [[ -n $next ]] && less "$current" "$next"; then
  parse "$next"
  ((ver_major > current_major)) && major=true
else
  next=""
fi

printf 'current=%s\nnext=%s\nmajor=%s\n' "$current" "$next" "$major"
