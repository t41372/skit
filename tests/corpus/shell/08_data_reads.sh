#!/usr/bin/env bash
while read -r line; do
  echo "$line"
done < input.txt
cat data.txt | while read -r item; do
  echo "$item"
done
read -r first < config.txt
echo "$first"
