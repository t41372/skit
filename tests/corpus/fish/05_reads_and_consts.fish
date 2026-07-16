#!/usr/bin/env fish
# v1 emits ONLY env-defaults: plain consts and `read` prompts are present here but must NOT
# surface as candidates (their delivery would need an injector fish doesn't have yet). The
# scanner must stay total and detect only the one genuine env-default.

set NAME world              # a plain const — NOT emitted in v1
set -x BUILD_DIR ./build    # an exported const — NOT emitted in v1

read -P 'Continue? ' answer         # a literal-prompt read — NOT emitted in v1
read -s -P 'Password: ' secret      # a secret read — NOT emitted in v1

set -q RETRIES; or set RETRIES 3    # the one real env-default — detected

echo "$NAME $BUILD_DIR $answer $RETRIES"
