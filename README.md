# Jasmine

## How performance is calculated in this work?

In our implmentation, there are three timestamps for a single transaction.

1. T1: A timestamp when transaction is created.
2. T2: A timestamp when block is packed by consensus node.
3. T3: A timestamp when it's finalized in a block.

End-to-end performance is calculated via T1 and T3, and 
Consensus performance is calculated via T2 and T3.

We mainly focus on consensus performance here, since it reflects how consensus
works.

## Usage

Generate config files for multi replicas over multi hosts.

Let's say we want to distribute 4 replicas over 2 hosts (IP_OF_SERVER_1, IP_OF_SERVER_2).

```Bash
# cargo r -- config-gen --help
cargo r -- config-gen --number 4 IP_OF_SERVER_1 IP_OF_SERVER_2 --export-dir configs -w
```

Now, some files are exported in `./configs`.

Then, distribute these files to corresponding servers.

**Please make sure you have right to login servers via `ssh IP_OF_SERVER`.**

```
cd ./configs

bash run-all.sh
```

This script will distribute configs, run replicas, and collect experiment results into your local directory.

## Test

### Distributes locally

```Bash
cargo r -- config-gen --number 4 localhost --export-dir configs -w
```

### Single replica via TCP:

```Bash
cargo run
```

### Replicas via memory network

```Bash
cargo run -- memory-test
```

### Failure test over memory network

```Bash
cargo run -- fail-test
```

## TODO

- [ ] Error handles
- [ ] crypto
