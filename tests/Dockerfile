ARG RUST_VERSION=1
FROM rust:${RUST_VERSION}-trixie
ENV DEBIAN_FRONTEND=noninteractive

RUN echo "deb [arch=$(dpkg --print-architecture)] http://download.proxmox.com/debian/pve trixie pve-no-subscription" > /etc/apt/sources.list.d/pve.list
ADD --chmod=644 https://enterprise.proxmox.com/debian/proxmox-release-trixie.gpg /etc/apt/trusted.gpg.d/
RUN apt-get update \
 && apt-get install --no-install-recommends -y pve-qemu-kvm \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/*
