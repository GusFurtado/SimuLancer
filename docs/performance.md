# Performance

How long a run takes, and how much memory it needs, as models grow. The numbers come from `bench/bench.py`, which anyone can rerun:

```sh
cargo build --release
python3 bench/bench.py            # the full table, a few minutes
python3 bench/bench.py --quick    # the two smallest cases
```

## The models

Each benchmark model is *K* parallel lines of *L* stations. One source feeds the lines in turn, every station fails now and then (a jam every 2 hours of work, 2 to 8 minutes to clear), and queues of 10 parts link the stations. Each station is busy about 85% of the time before failures, so parts wait, block and starve as in a real line. Every run is **one replication on one thread**, after a one-day warm-up, so the times are per replication: a 20-replication experiment on 8 cores takes about 20 / 8 times as long.

## Results

Measured on one core of a 6-core desktop processor from 2020, on Linux, on 2026-11-27.

| case | objects | load (s) | one replication (s) | parts/h | events/s | peak memory (MB) | event log (MB) |
|---|---|---|---|---|---|---|---|
| objects-100 | 103 | 0.01 | 0.39 | 593 |  | 12 |  |
| objects-1000 | 1,003 | 0.03 | 5.57 | 1,466 | 4,247,756 | 36 |  |
| objects-10000 | 10,003 | 0.72 | 157.00 | 5,656 |  | 281 |  |
| wip-high | 1,003 | 0.03 | 7.62 | 1,444 |  | 42 |  |
| length-1d | 1,003 | 0.03 | 1.42 | 1,457 |  | 36 |  |
| length-28d | 1,003 | 0.03 | 20.18 | 1,463 |  | 36 |  |
| event-log | 1,003 | 0.03 | 13.31 | 1,466 | 1,777,120 (with the log) | 36 | 1849 |

- **Objects:** stations, queues, the source and the sink.
- **Load:** reading and checking the model (`simulancer validate`).
- **Events per second:** trace events (a part arriving, starting, leaving...) in the model of 1,000 objects, counted from its event log, over the time of the same run without the log.

## What it says

- **Loading is never the problem:** 0.7 s for 10,000 objects.
- **Time grows with the work:** parts moved through stations. A model of 1,000 objects runs 8 simulated days in 5.6 s per replication, about 4 million events per second. Run length is linear: 1.4 s for 2 simulated days, 5.6 s for 8, 20 s for 29.
- **Very large models cost more per event.** 10,000 objects do about 10 times the work of 1,000 in 28 times the time, and need 281 MB. Their state no longer fits in the processor's caches, and the event list is ten times longer. A replication still takes under three minutes on one core, and replications run in parallel.
- **More parts in the system cost little:** with unlimited queues at 97% load, 7.6 s instead of 5.6 s, and 42 MB.
- **The event log is the expensive option:** it makes the run 2.4 times slower and wrote 1.8 GB for 8 days of 1,000 objects. It's only written for the replication the animation replays, and only when the model asks for it (`outputs.event_log: jsonl`); leave it off for large experiments.

## Fixed along the way

The first run of this benchmark found that a replication's time grew with the *square* of the model's size: 1,000 objects took 149 s where 100 took 2. After every event, the engine brought every server's and conveyor's time averages up to date, which made each event cost as much as the model was big. Now an event updates only the objects it changed. In debug builds, every test checks that no other object changed, and results are bit-identical to before. The same 1,000 objects now take 5.6 s.
