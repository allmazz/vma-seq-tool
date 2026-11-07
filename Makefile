PODMAN_TEST_IMAGE = allmazz/vma-seq-tool-test

.PHONY: test test_in_podman build clean

build:
	cargo build --release

test:
	cargo test

test_in_podman:
	podman build -t $(PODMAN_TEST_IMAGE) ./tests
	podman run --rm -v "$(CURDIR):/vma-seq-tool:Z" -w /vma-seq-tool $(PODMAN_TEST_IMAGE) \
		cargo test --target-dir target/podman

install: build
	sudo install -Dm755 target/release/vma-seq-tool /usr/bin/vma-seq-tool

clean:
	rm -rf target
