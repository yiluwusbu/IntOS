FROM ubuntu:22.04
ENV DEBIAN_FRONTEND noninteractive
# Install build dependencies
RUN apt-get update && apt-get -y install curl wget p7zip-full libncurses5 libncursesw5 build-essential qemu-system-arm \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    &&  . "$HOME/.cargo/env" && rustup toolchain add nightly-2022-04-01

# Install MSP toolchain
RUN wget https://dr-download.ti.com/software-development/ide-configuration-compiler-or-debugger/MD-LlCjWuAbzH/9.3.1.2/msp430-gcc-full-linux-x64-installer-9.3.1.2.7z \
    && p7zip -d msp430-gcc-full-linux-x64-installer-9.3.1.2.7z && ./msp430-gcc-full-linux-x64-installer-9.3.1.2.run --prefix "/ti" --mode unattended \
    && echo "export PATH=\$PATH:/ti/bin" >> /root/.bashrc

WORKDIR /repo