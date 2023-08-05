# Foxglove Bridge

[![Rust](https://github.com/dmweis/foxglove-bridge/workflows/Rust/badge.svg)](https://github.com/dmweis/foxglove-bridge/actions)
[![Private docs](https://github.com/dmweis/foxglove-bridge/workflows/Deploy%20Docs%20to%20GitHub%20Pages/badge.svg)](https://davidweis.dev/foxglove-bridge/foxglove_bridge/index.html)

## Install protoc on ubuntu

```bash
sudo apt update && sudo apt install autoconf automake libtool curl make g++ unzip -y
```

1. Download from [github releases](https://github.com/protocolbuffers/protobuf/releases)
2. extract `tar -xvf`
3. `./configure`
4. `make -j16`
5. `make -j16 check`
6. `sudo make install`
7. `sudo ldconfig`
