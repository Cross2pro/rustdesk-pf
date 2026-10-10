#!/bin/sh
# User-mode (no sudo) launcher for rustdesk-pf serve. No secrets here:
# put them in ~/.config/rustdesk-pf/rustdesk-pf.env (chmod 600).
set -eu
DIR="${RUSTDESK_PF_HOME:-$HOME/.config/rustdesk-pf}"
BIN="${RUSTDESK_PF_BIN:-$HOME/bin/rustdesk-pf}"
ENV="$DIR/rustdesk-pf.env"; PID="$DIR/serve.pid"; LOG="$DIR/serve.log"
running() { [ -f "$PID" ] && kill -0 "$(cat "$PID")" 2>/dev/null; }
start() {
  running && { echo "already running (pid $(cat "$PID"))"; return; }
  [ -f "$ENV" ] || { echo "missing $ENV" >&2; exit 1; }
  chmod 600 "$ENV"
  set -a; . "$ENV"; set +a
  export RUSTDESK_PF_CONFIG="${RUSTDESK_PF_CONFIG:-$DIR/state}"
  mkdir -p "$RUSTDESK_PF_CONFIG"
  nohup "$BIN" serve >>"$LOG" 2>&1 &
  echo $! >"$PID"; echo "started (pid $!), log: $LOG"
}
stop() { running && kill "$(cat "$PID")" && echo stopped; rm -f "$PID"; }
case "${1:-start}" in
  start) start ;; stop) stop ;; restart) stop || true; sleep 1; start ;;
  status) running && echo "running (pid $(cat "$PID"))" || echo "not running" ;;
  log) tail -f "$LOG" ;;
  *) echo "usage: $0 start|stop|restart|status|log" >&2; exit 2 ;;
esac
