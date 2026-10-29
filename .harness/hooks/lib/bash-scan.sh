#!/usr/bin/env bash
# Общая часть Bash-хуков. Подключается через `. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"`
# — путь берётся от самого скрипта, чтобы хук работал и в рабочем дереве сессии.
#
# Вырезание heredoc'ов — не украшение, а урок: первая версия защитного хука
# заблокировала запись документации, в тексте которой упоминались опасные
# команды. Хук, срабатывающий на рассказ о себе, обучает себя обходить.
#
# Ошибка разбора обязана быть в сторону запрета: вырезанное хук не видит.
# Охранные хуки защищают от ошибки, а не от злонамеренного агента, поэтому
# полного лексера bash здесь нет — есть правило «сомневаешься — не вырезай»:
# - heredoc — только `<<` вне кавычек, вне арифметики (`((…))`, `$((…))`,
#   `$[…]`, `let …`) и вне комментария; `<<<` — here-string, не heredoc;
# - из терминатора снимаются только кавычки; `\` или `$` в нём (включая
#   `$'…'` и `$"…"`) — сомнение;
# - ведущие табы в строке терминатора снимаются только при `<<-`;
# - сомнение также — `$'` или `$"` в строке, кавычка внутри арифметики или
#   `let`, строка, оканчивающаяся неэкранированным `\`;
# - сомнение или heredoc без закрывающей строки — команда сканируется целиком.

# Читает событие хука со stdin и печатает команду с вырезанными телами heredoc'ов.
# Возвращает 1, если команды в событии нет.
bash_scan() {
  local cmd
  cmd=$(jq -r '.tool_input.command // empty' 2>/dev/null) || return 1
  [ -n "$cmd" ] || return 1
  printf '%s\n' "$cmd" | awk '
    BEGIN { sq = sprintf("%c", 39); dq = "\""; bs = "\\"; inq = ""; full = 0; pending = 0 }
    {
      all[NR] = $0
      if (pending) {
        line = $0
        if (tabs[cur]) sub(/^\t+/, "", line)
        if (line == terms[cur]) {
          cur++
          if (cur > nterms) pending = 0
        }
        next
      }
      out[++n] = $0

      s = $0
      L = length(s)
      i = 1
      nterms = 0
      cmdstart = 1
      # Сомнение: строки $'…' и $"…" и продолжение строки неэкранированным `\`.
      if (index(s, "$" sq) || index(s, "$" dq)) full = 1
      if (match(s, /\\+$/) && RLENGTH % 2 == 1) full = 1
      while (i <= L) {
        c = substr(s, i, 1)
        # Кавычки: состояние переживает перевод строки.
        if (inq == sq) { if (c == sq) inq = ""; i++; continue }
        if (inq == dq) {
          if (c == bs) { i += 2; continue }
          if (c == dq) inq = ""
          i++
          continue
        }
        if (c == bs) { i += 2; cmdstart = 0; continue }
        if (c == sq || c == dq) { inq = c; i++; cmdstart = 0; continue }
        # Комментарий до конца строки.
        if (c == "#" && (i == 1 || substr(s, i - 1, 1) ~ /[ \t;|&(]/)) break
        # Арифметика: сдвиг << в ней не heredoc. Незакрытая — до конца строки.
        # Кавычка внутри пропускаемой области — сомнение: состояние кавычек
        # дальше не отследить.
        if (substr(s, i, 3) == "$((" || substr(s, i, 2) == "((") {
          o = (substr(s, i, 1) == "$") ? 3 : 2
          j = index(substr(s, i + o), "))")
          area = (j == 0) ? substr(s, i + o) : substr(s, i + o, j - 1)
          if (index(area, sq) || index(area, dq)) full = 1
          if (j == 0) break
          i = i + o + j + 1
          cmdstart = 0
          continue
        }
        if (substr(s, i, 2) == "$[") {
          j = index(substr(s, i + 2), "]")
          area = (j == 0) ? substr(s, i + 2) : substr(s, i + 2, j - 1)
          if (index(area, sq) || index(area, dq)) full = 1
          if (j == 0) break
          i = i + 2 + j
          cmdstart = 0
          continue
        }
        if (cmdstart && substr(s, i, 3) == "let" && substr(s, i + 3, 1) ~ /[ \t]/) {
          i += 3
          while (i <= L && substr(s, i, 1) !~ /[;|&]/) {
            if (substr(s, i, 1) == sq || substr(s, i, 1) == dq) full = 1
            i++
          }
          continue
        }
        if (c ~ /[;|&(]/) { cmdstart = 1; i++; continue }
        if (c == " " || c == "\t") { i++; continue }
        if (substr(s, i, 2) == "<<") {
          if ((i > 1 && substr(s, i - 1, 1) == "<") || substr(s, i + 2, 1) == "<") {
            while (substr(s, i, 1) == "<") i++
            cmdstart = 0
            continue
          }
          k = i + 2
          dash = 0
          if (substr(s, k, 1) == "-") { dash = 1; k++ }
          while (substr(s, k, 1) == " " || substr(s, k, 1) == "\t") k++
          w = ""
          wq = ""
          while (k <= L) {
            d = substr(s, k, 1)
            if (wq != "") { if (d == wq) wq = ""; w = w d; k++; continue }
            if (d == sq || d == dq) { wq = d; w = w d; k++; continue }
            if (d ~ /[ \t;|&<>()]/) break
            w = w d
            k++
          }
          if (wq != "" || index(w, bs) || index(w, "$")) {
            full = 1
          } else {
            gsub(sq, "", w)
            gsub(dq, "", w)
            if (w != "") {
              nterms++
              terms[nterms] = w
              tabs[nterms] = dash
            }
          }
          i = k
          cmdstart = 0
          continue
        }
        cmdstart = 0
        i++
      }
      if (nterms > 0) { pending = 1; cur = 1 }
    }
    END {
      if (pending || full) {
        for (i = 1; i <= NR; i++) print all[i]
      } else {
        for (i = 1; i <= n; i++) print out[i]
      }
    }
  '
}

# Отказ PreToolUse с причиной. Причина уходит модели, поэтому в ней —
# что делать вместо, а не только запрет.
deny() {
  jq -nc --arg r "$1" '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: $r
    }
  }'
  exit 0
}
