#!/bin/bash
export API_HOST=example.com
export PORT=8080
readonly MAX=100
declare -r LOCKED=yes
declare -i COUNT=5
typeset FLAVOR=vanilla
local NOPE=nope
echo "$API_HOST $PORT $MAX $COUNT $FLAVOR"
