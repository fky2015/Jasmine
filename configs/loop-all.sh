#!/bin/bash

#################################
# 
# This script loops through all the files (generate by config-gen) in a directory
# and runs a command on each file.
#
# This helps to continuously conduct a series of experiments.
#
# Features:
# 1. The script will find all the runnable folder in the current directory,
#   and run the command by the order of creation time.
# 2. All the results are collected to current directory.
# 3. When timeout, the script will auto retry the experiment.
#
# Usage: bash loop-all.sh
#
# Author: Feng Kaiyu <loveress01@outlook.com>
#
#################################

# Kill current process and retry after $TIMEOUT seconds.
# This should be larger than the time it takes to run the script normally.
TIMEOUT=600

# No need to change this
FILE="run-all.sh"

# Run the script, and remove it if it succeeds.
cmd() {
  NEXT_FOLDER=$1

  echo "cd $NEXT_FOLDER"
  cd "$NEXT_FOLDER" || exit 1

  echo "Running $FILE"
  bash "$FILE"

  echo "cd .."
  cd ..
  
  # move result to folder
  find "$NEXT_FOLDER" -type f -name "result*.json" -exec mv '{}' . \;

  # remove folder
  rm -r "$NEXT_FOLDER"
}

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

  # Wait until timeout
  cmd "$NEXT_FOLDER" &
  sleep $TIMEOUT &
  wait -n

  # Kill all children, continue
  pkill -P $$
}

while true; do
  find_dir

  echo Sleeping 5 seconds
  sleep 5
done
