#!/bin/bash
set -e

echo "Updating apt and installing wrk and procps..."
apt-get update -y > /dev/null 2>&1
apt-get install -y wrk procps > /dev/null 2>&1

echo "Building rest_runtime..."
(cd rest_runtime && cargo build)

echo "Compiling web_magic.rest..."
cargo run -- build examples/web_magic.rest -o web_magic_bin

echo "Starting server..."
./web_magic_bin &
SERVER_PID=$!
sleep 2

echo "=== Memory before test ==="
ps -o pid,rss,vsz,comm -p $SERVER_PID

echo "=== Running wrk for 10 seconds (Connection: close) ==="
wrk -t4 -c100 -d10s -H "Connection: close" http://127.0.0.1:8080/hello
echo "=== Memory after test ==="
ps -o pid,rss,vsz,comm -p $SERVER_PID

kill $SERVER_PID
