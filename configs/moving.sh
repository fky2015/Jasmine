#!/bin/bash

####################################
#
# moving.sh
#
# This script moves results from current directory to destination, 
# autoraname the files if conflict.
#
# Usage: moving.sh <destination directory>
#
####################################

DEST=${1:-../data/5-plans}

echo "moving destination ${DEST}"

while true; do
  if compgen -G "result*.json" > /dev/null; then
    for f in $(find . -maxdepth 1 -type f -name "result*.json"); do
      basename=$(basename "$f")
      extention="${basename##*.}"
      filename="${basename%.*}"
      # if destination file exists, append a number to the filename
      if [[ -f "$DEST/$basename" ]]; then
        i=1
        while [[ -f "$DEST/$filename--$i.$extention" ]]; do
          let i++
        done
        basename="$filename--$i.$extention"
      fi
      echo "moving $f to $DEST/$basename"
      mv "$f" "$DEST/$basename"
    done
  else
    sleep 2
  fi
done
