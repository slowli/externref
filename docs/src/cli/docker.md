# Docker Image

As a lower-cost alternative to the local installation, you may install and use the CLI app
from the [GitHub Container registry](https://github.com/slowli/externref/pkgs/container/externref).
To run the app in a Docker container, use a command like

```bash
docker run -i --rm ghcr.io/slowli/externref:main - \
  < module.wasm \
  > processed-module.wasm
```

Here, `-` is the argument to the CLI app instructing to read the input module from the stdin.
To output tracing information, set the `RUST_LOG` env variable in the container,
e.g. using `docker run --env RUST_LOG=debug ...`.
