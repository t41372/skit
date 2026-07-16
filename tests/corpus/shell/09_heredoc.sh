#!/usr/bin/env bash
TEMPLATE_NAME=report
cat <<EOF
Hello, this is a heredoc.
Line two mentions $TEMPLATE_NAME and read x here.
EOF
echo "$TEMPLATE_NAME"
