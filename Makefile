.PHONY: all clean

all: http_server
http_server:
	rustc src/main.rs \
    --crate-name http_server \
    --edition 2021 \
    --crate-type bin \
    -C opt-level=3 \
    -C lto=thin \
    -C strip=debuginfo \
    -C codegen-units=1 

clean:
	rm http_server
