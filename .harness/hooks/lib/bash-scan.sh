#!/usr/bin/env bash
# Shared part of the Bash hooks. Sourced via `. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"`
# — the path is taken from the script itself so the hook works in a session worktree too.
#
# Cutting out heredocs is not decoration but a lesson: the first version of the
# guard hook blocked writing documentation whose text mentioned dangerous
# commands. A hook that fires on a story about itself teaches agents to bypass it.
#
# A parse error must err on the side of denial: the hook doesn't see what is cut out.
# Guard hooks protect against mistakes, not against a malicious agent, so there
# is no full bash lexer here — there is the rule "in doubt, don't cut":
# - a heredoc is only `<<` outside quotes, outside arithmetic (`((…))`, `$((…))`,
#   `$[…]`, `let …`) and outside a comment; `<<<` is a here-string, not a heredoc;
# - only quotes are stripped from the terminator; a `\` or `$` in it (including
#   `$'…'` and `$"…"`) is doubt;
# - leading tabs in the terminator line are stripped only with `<<-`;
# - also doubt: `$'` or `$"` in the line, a quote inside arithmetic or
#   `let`, a line ending with an unescaped `\`;
# - on doubt or a heredoc without a closing line, the whole command is scanned.

# Reads the hook event from stdin and prints the command with heredoc bodies cut out.
# Returns 1 if the event has no command.
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
      # Doubt: $'…' and $"…" strings and line continuation with an unescaped `\`.
      if (index(s, "$" sq) || index(s, "$" dq)) full = 1
      if (match(s, /\\+$/) && RLENGTH % 2 == 1) full = 1
      while (i <= L) {
        c = substr(s, i, 1)
        # Quotes: the state survives a newline.
        if (inq == sq) { if (c == sq) inq = ""; i++; continue }
        if (inq == dq) {
          if (c == bs) { i += 2; continue }
          if (c == dq) inq = ""
          i++
          continue
        }
        if (c == bs) { i += 2; cmdstart = 0; continue }
        if (c == sq || c == dq) { inq = c; i++; cmdstart = 0; continue }
        # Comment to the end of the line.
        if (c == "#" && (i == 1 || substr(s, i - 1, 1) ~ /[ \t;|&(]/)) break
        # Arithmetic: a << shift in it is not a heredoc. Unclosed — to the end of the line.
        # A quote inside the skipped area is doubt: the quote state cannot be
        # tracked further.
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

# A PreToolUse denial with a reason. The reason goes to the model, so it says
# what to do instead, not just the prohibition.
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
