#!/bin/sh

PREV_TMUX_COMMAND=`cat /tmp/prev-tmux-command`
echo $PREV_TMUX_COMMAND
tmux send-keys -t 1 $PREV_TMUX_COMMAND Enter