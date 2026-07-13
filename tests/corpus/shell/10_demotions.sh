#!/usr/bin/env bash
COUNT=0
TOTAL=100
STEP=5
COUNT+=1
TOTAL=$((TOTAL - STEP))
((COUNT++))
let STEP=STEP+1
SUM=0
for i in 1 2 3; do
  SUM=$((SUM + i))
done
echo "$COUNT $TOTAL $STEP $SUM"
