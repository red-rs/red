#!/bin/sh

PANES=$(tmux list-panes | wc -l)

if [ "$PANES" -le 1 ]; then
  tmux split-window -v
fi

tmux send-keys -t 1 "$@" Enter
echo "$1" > /tmp/prev-tmux-command