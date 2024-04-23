default: build

build:
	RUSTFLAGS=-Awarnings cargo build --release

flamegraph:
	sudo cargo flamegraph  -- test/testb.py

bloat:
	cargo bloat --crates --release

build-timings:
	cargo build --timings --release

depgraph:
	cargo depgraph | dot -Tpng > graph.png

docker:
	docker build -f .dockerfile -t red:latest .

docker-run:
	docker run -it --rm red:latest
