
- `1-plain`: The LT graph (Latency-Throughput graph, same below) in `batch_size = 250, 500, 1000` in WAN.
- `2-plain`: The LT graph in `transaction_size = 128, 256, 512` in WAN.
- `3-plain`: The LT graph in `network delay = 0 (localhost), 100, 500`.
- `4-plain`: The LT graph under fault `f = 1, 2, 5` in WAN.
- `5-plain`: The LT graph under `leader_rotation = 1, 10, 100` in WAN.

In theoretic analysis, best case is `batch_size = 1000, transaction_size = 128, leader_rotation = 100`
