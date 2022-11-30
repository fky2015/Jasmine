#!/bin/bash


while [[ true ]]; do
  if compgen -G "result*.json" > /dev/null; then
    echo "moving"
    mv result*.json ../data/5-plans/
  else
    sleep 2
  fi
done
