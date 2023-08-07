# Jasmine [^1]

## Quick Start

Directly executing a binary will start a committee composed of a single node using localhost TCP for communication.

```Bash
cargo r
```

## Config Generation

The subcommand `config-gen` provide a way to generate multiple files for multi replicas over multi hosts.
It also helps to generate a bunch of bash scripts to distribute files in accordance, run all the nodes, and
collect the results.

Please lookup the document of config-gen before using it.

`cargo r -- config-gen --help`

**Remember that, default `config-gen` will run in dry-run mode, in which all the files will be print to the console.
By specify `-w` you can flush these files to disks.**

Let's say we want to distribute 4 replicas over 2 hosts (IP_OF_SERVER_1, IP_OF_SERVER_2).

```Bash
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

## Testing

### How is the test work flow?

First, we can try to export a basic config file. (This can be optional)
And you can edit further `base_config.json` if you like.

```
cargo r -- --export-path base_config.json
```

Next, You will use `config-gen` to generate all the files for a committee in a single tests.

```
cargo r -- --config base_config.json config-gen --number <NUMBER> IPs_OF_SERVER --export-dir configs -w
```

If you skip the first step, then just run (default config will be used):

```
cargo r -- config-gen --number <NUMBER> IPs_OF_SERVER --export-dir configs -w
```

As the section above, run:

```
cd configs/
bash run-all.sh
```

Then you'll get the results.

### How performance is calculated in this work?

In our implmentation, there are three timestamps for a single transaction.

1. T1: A timestamp when transaction is created.
2. T2: A timestamp when block is packed by consensus node.
3. T3: A timestamp when it's finalized in a block.

End-to-end performance is calculated via T1 and T3, and 
Consensus performance is calculated via T2 and T3.

We mainly focus on e2e performance here, 
and since consensus throughput is same as end-to-end throughput, we calculate the throughput
in consensus instead.

### Some local test methods.

#### Distributes locally

```Bash
cargo r -- config-gen --number 4 localhost --export-dir configs -w
```

#### Single replica via TCP:

```Bash
cargo run
```

#### Replicas via memory network

```Bash
cargo run -- memory-test
```

#### Failure test over memory network

```Bash
cargo run -- fail-test
```

[^1]: This is dedicated to my girlfriend Jasmine Jiang.
