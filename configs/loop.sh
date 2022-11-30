#!/bin/bash
file="run-all.sh"
while true; do
  if [[ -f $file ]]; then
    echo "File exists"
    bash $file
    rm $file 
  else 
    sleep 1
  fi
done
