.PHONY: all clean run

all:
	cargo build --release

clean:
	cargo clean

run:
	cargo run
