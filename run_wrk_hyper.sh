#!/bin/bash
set -e

echo "Updating apt and installing wrk and procps..."
apt-get update -y > /dev/null 2>&1
apt-get install -y wrk procps > /dev/null 2>&1

echo "Compiling web_magic.rest..."
cargo run -- build examples/web_magic.rest -o web_magic_bin > /dev/null 2>&1

echo "Starting server..."
./web_magic_bin &
SERVER_PID=$!
sleep 2

echo "=== Memory before test ==="
ps -o pid,rss,vsz,comm -p $SERVER_PID

echo "=== Running wrk for 10 seconds (Keep-Alive) ==="
wrk -t4 -c100 -d10s http://127.0.0.1:8080/hello
echo "=== Memory after test ==="
ps -o pid,rss,vsz,comm -p $SERVER_PID

echo "=== Running wrk for 15 seconds (Keep-Alive) ==="
wrk -t4 -c100 -d15s http://127.0.0.1:8080/hello
echo "=== Memory after second test ==="
ps -o pid,rss,vsz,comm -p $SERVER_PID

kill $SERVER_PID
