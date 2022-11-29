#!/bin/bash

P=../../../../ghq/github.com/asonnino/narwhal/benchmark

RATE=$(grep -oP 'RATE="\${3:-\K\w+' $P/start.sh)
SIZE=$(grep -oP 'SIZE="\${4:-\K\w+' $P/start.sh)

echo "{\"input\": {\"consensus\": \"Narwhal\", \"parameters\": `cat $P/.parameters.json`, \"client\": {\"injection_rate\": $RATE}} }" | \
  jq '.input.parameters += {"transaction_size": ($SIZE | tonumber) }' --arg SIZE $SIZE \
| jq '. += {"output": {"throughput": 0, "latency": 0 }}'
