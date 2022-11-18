#!/bin/bash
# https://daniel.haxx.se/blog/2010/12/14/add-latency-to-localhost/
#
tc qdisc add dev lo root handle 1:0 netem delay 100msec


tc qdisc del dev lo root
