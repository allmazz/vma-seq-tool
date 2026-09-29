PODMAN_TEST_IMAGE = allmazz/vma-seq-tool-test

.PHONY: test_in_podman build clean

build:
	cargo build --release

test_in_podman: build
	podman build -t $(PODMAN_TEST_IMAGE) ./tests
	podman run -it --rm -v .:/vma-seq-tool $(PODMAN_TEST_IMAGE) /vma-seq-tool/tests/test.sh \
		/vma-seq-tool/target/release/vma-seq-tool \
		/vma-seq-tool/tests/tmp \
		/vma-seq-tool/tests/samples

install: build
	sudo install -Dm755 target/release/vma-seq-tool /usr/bin/vma-seq-tool

clean:
	rm -rf target
