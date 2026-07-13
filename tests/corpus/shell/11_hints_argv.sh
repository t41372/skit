#!/usr/bin/env bash
while getopts "n:v" opt; do
  case "$opt" in
    n) echo "name: $OPTARG" ;;
    v) echo "verbose" ;;
  esac
done
shift $((OPTIND - 1))
echo "First: $1  All: $@  Count: $#  Star: $*"
