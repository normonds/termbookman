#!/bin/bash
# Wrapper to run termbookman with a timeout
./termbookman &
PID=$!
sleep 10
kill $PID
