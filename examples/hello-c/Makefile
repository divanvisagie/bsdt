# Works with both FreeBSD make and GNU make.

hello: hello.c
	$(CC) -Wall -O2 -o hello hello.c

run: hello
	./hello

# Answer HTTP requests on port 8080 with hello's output until interrupted.
serve: hello
	@echo "serving on :8080, try: curl 127.0.0.1:18080 (ctrl-c to stop)"
	@while true; do (printf 'HTTP/1.0 200 OK\r\n\r\n'; ./hello) | nc -N -l 8080 >/dev/null; done

clean:
	rm -f hello

.PHONY: run serve clean
