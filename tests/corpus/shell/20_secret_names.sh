#!/usr/bin/env bash
API_KEY=changeme
read -s PASSWORD
echo "${SECRET_TOKEN:-none}"
echo "$API_KEY $PASSWORD"
