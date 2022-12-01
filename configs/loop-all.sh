#!/bin/bash
#
FILE="run-all.sh"

set -x

find_dir ()
{
  RES=$(find . -maxdepth 2 -mindepth 2 -type f -name "*$FILE" -printf "%T@ %p\n" |sort -n|awk '{print $2}' | sed -r 's|/[^/]+$||'|uniq )
  
  FOLDERS=($RES)

  if [ ${#FOLDERS[@]} -eq 0 ]; then
    echo "No folders found"
    return
  fi

  echo "Found ${#FOLDERS[@]} folders with $FILE"

  NEXT_FOLDER=${FOLDERS[0]}

  # execute

  cd "$NEXT_FOLDER" || exit 1

  bash "$FILE"

  cd ../

  # move result to folder
  find "$NEXT_FOLDER" -type f -name "result*.json" -exec mv '{}' . \;

  # remove folder
  rm -r "$NEXT_FOLDER"
}

while true; do
  find_dir

  echo Sleeping 5 seconds
  sleep 5
done
