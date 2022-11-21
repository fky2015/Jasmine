# Jasmine

## How performance is calculated in this work?

In our implmentation, there are three timestamps for a single transaction.

1. T1: A timestamp when transaction is created.
2. T2: A timestamp when block is packed by consensus node.
3. T3: A timestamp when it's finalized in a block.

End-to-end performance is calculated via T1 and T3, and 
Consensus performance is calculated via T2 and T3.


## TODO

- [ ] Error handles
- [ ] crypto
- [ ] exports metrics / benchmarks
- [ ] e2e test in WAN
- [ ] batch exports config files.
