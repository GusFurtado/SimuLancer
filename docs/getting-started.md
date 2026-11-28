# Getting started

SimuLancer is one program, `simulancer`, with four commands: `validate` and `run` for a model, `compare` for two of them, and `serve` for the web app.

## Install

=== "Download (Linux)"

    Each release on the [releases page](https://github.com/GusFurtado/SimuLancer/releases) has a static Linux binary (x86_64) packed with the examples, the documentation and the deploy files. Set `v` to the latest version listed there:

    ```sh
    v=v0.1.0
    base=https://github.com/GusFurtado/SimuLancer/releases/download/$v
    curl -LO $base/simulancer-$v-x86_64-linux.tar.gz
    curl -LO $base/simulancer-$v-x86_64-linux.tar.gz.sha256
    sha256sum -c simulancer-$v-x86_64-linux.tar.gz.sha256
    tar xzf simulancer-$v-x86_64-linux.tar.gz
    cd simulancer-$v-x86_64-linux
    ./simulancer --version
    ```

    Copy `simulancer` anywhere on your `PATH`. `serve` also needs git on the system (`sudo apt install git` or your distribution's package), since it versions projects with it.

=== "Windows"

    Download `simulancer-<version>-x86_64-windows.zip` from the [releases page](https://github.com/GusFurtado/SimuLancer/releases) and unzip it. It has `simulancer.exe`, the examples and the documentation. In PowerShell, in that folder:

    ```powershell
    .\simulancer.exe --version
    ```

    Windows may warn that the program is from an unknown publisher (it isn't code-signed): choose *More info → Run anyway*. The web app (`serve`) also needs [Git for Windows](https://git-scm.com/download/win), since it versions projects with git.

=== "macOS"

    Download `simulancer-<version>-aarch64-macos.tar.gz` (Apple silicon) or `-x86_64-macos.tar.gz` (Intel) from the [releases page](https://github.com/GusFurtado/SimuLancer/releases), then:

    ```sh
    tar xzf simulancer-*-macos.tar.gz
    cd simulancer-*-macos
    xattr -d com.apple.quarantine simulancer   # it isn't notarized by Apple
    ./simulancer --version
    ```

    git comes with Apple's command line tools (`xcode-select --install`).

=== "Docker"

    ```sh
    docker run -d --name simulancer -p 127.0.0.1:8080:8080 -v simulancer-data:/data ghcr.io/gusfurtado/simulancer:latest
    ```

    The image runs the web app and includes git. Open <http://localhost:8080>.

=== "From source"

    You need a recent stable Rust (install it with [rustup](https://rustup.rs)) and git. It builds on Linux, Windows and macOS.

    ```sh
    git clone https://github.com/GusFurtado/SimuLancer.git
    cd SimuLancer
    cargo build --release
    ./target/release/simulancer --version
    ```

    The binary is `target/release/simulancer`. Copy it anywhere on your `PATH`.

Running it as a service (systemd, HTTPS, logins) is in [Deploying](deploy.md).

### On your own computer

No server is needed to use the web app alone: run `simulancer serve` in a folder of your choice and open <http://localhost:8080> in your browser. It keeps its projects in a `data` folder there (`--data` picks another). Only your computer can reach it, since it listens on `127.0.0.1`. Stop it with Ctrl+C.

## Run an example

The `examples/` folder has ready-made models. Check one first: `validate` prints what it understood, so you can see whether the model says what you meant.

```console
$ simulancer validate examples/conveyor-line.yaml
ok: examples/conveyor-line.yaml (conveyor-line)

sources
  arrivals     creates part, every exponential(1 min), mean 1 min

servers
  lathe        capacity 1, process triangular(30 s, 48 s, 1.2 min), mean 50 s
                 fails (breakdown) after exponential(45 min), mean 45 min, down for uniform(4 min, 12 min), mean 8 min
  mill         capacity 1, process triangular(36 s, 54 s, 1.1 min), mean 52 s
                 fails (breakdown) after exponential(1 h), mean 1 h, down for lognormal(mean 10 min, sd 4 min), mean 10 min

conveyors
  c1           arrivals -> lathe    12 m at 0.5 m/s, holds 24 parts, transit 24 s
  c2           lathe -> mill        8 m at 0.5 m/s, holds 16 parts, transit 16 s
  c3           mill -> shipping     6 m at 0.5 m/s, holds 12 parts, transit 12 s

experiment: 30 replications, warm-up 8 h, run length 5 d, seed 20261009
```

Then run it. Replications run in parallel on all cores, and every result is a mean with its 95% confidence interval:

```console
$ simulancer run examples/conveyor-line.yaml
conveyor-line: 30 replications × 5 d after 8 h warm-up, seed 20261009 (0.05 s on 12 threads)

                       mean      95% CI        min        max
source arrivals
  blocked              7.2%      ± 0.5%       4.0%      10.4%
server lathe
  idle                 2.6%      ± 0.3%       1.2%       4.4%
  busy                77.6%      ± 0.4%      74.7%      79.2%
  ...
sink shipping
  throughput        55.9 /h    ± 0.3 /h    53.6 /h    57.2 /h
  time in system  25.76 min  ± 0.54 min  22.72 min  29.20 min

warning: the warm-up (8 h) may be too short: the parts in the system settle after about 10.67 h (see the report's warm-up check)
```

The warning comes from the [warm-up check](methods/warmup.md). The results are in `results/conveyor-line/`, and `report.html` is the place to start. See [Results](results.md).

## Compare two models

`examples/conveyor-line-longer-c2.yaml` is a [scenario](scenarios.md): the same line with a conveyor twice as long. `compare` runs both on the same random numbers and reports each difference with its confidence interval:

```console
$ simulancer compare examples/conveyor-line.yaml examples/conveyor-line-longer-c2.yaml
...
sink shipping
  throughput        55.9 /h    57.0 /h     +1.1 /h    ± 0.1 /h  higher
  time in system  25.76 min  30.80 min   +5.04 min  ± 0.51 min  higher

Common random numbers made the confidence intervals 5.9× narrower (median) than independent runs would.
```

A longer buffer ships 1.1 more parts per hour, but each part spends 5 minutes longer in the system. See [Comparing scenarios](methods/comparisons.md).

## Open the web app

```sh
simulancer serve --data ./data
```

Open <http://localhost:8080>. A new data folder has no projects: start one blank or from a template, then follow the [tutorial](tutorial.md). The palette button at the top right of every page picks one of six color themes (Crimson Dragoon by default), light or dark; by default the mode follows the system.
