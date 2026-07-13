#!/usr/bin/env bash
# The function's read is FIRST in source order but runs LAST: a runtime call counter would
# swap the two values (handing the password to "Name:"); call-site binding cannot.
ask_secret() {
  read -s -p "Password: " PW
  echo
}
read -p "Name: " NAME
ask_secret
echo "name=$NAME pw-len=${#PW}"
