# Stock and restocking

A store of parts (pallets in racks, kits on shelves, beds, spares) runs down as it's used and is refilled by orders. A source can keep a **queue** stocked instead of creating parts on a clock.

```yaml
format: simulancer/0.1
name: kanban-shelf
entities:
  kit: {}
  order: {}
  shipment: {}
sources:
  supplier:
    creates: kit
    restocks: shelf
    reorder_point: 20            # order when the stock position falls to 20
    order: 40                    # a box of 40 kits
    lead_time: "uniform(1 d, 3 d)"
  demand: { creates: order, interarrival: "exponential(10 min)" }
servers:
  pick:
    assembles: { kit: 1, order: 1 }
    makes: shipment
    keeps_clock_of: order        # time in system counts from the order
    process_time: "2 min"
    input: { rule: first_come }
queues:
  shelf: { from: supplier, to: pick, capacity: 100, starts_with: 60 }
  backlog: { from: demand, to: pick }
  out: { from: pick, to: shipping }
sinks:
  shipping: {}
experiment:
  replications: 20
  warmup: "7 d"
  run_length: "56 d"
  seed: 1
```

## Choices

- **Continuous or periodic review.** By default the source checks the stock every time a part leaves the queue. `review_every: "1 d"` checks once a day instead, like a planner who orders on Mondays: it needs a higher reorder point to cover the days between checks.
- **Fixed quantity or order-up-to.** `order: 40` orders boxes of 40 (several if one isn't enough to get above the reorder point). `order_up_to: 100` orders whatever brings the position back to 100.
- **The stock position** counts what's on the shelf, what's been delivered but hasn't got there yet, and what's on order. Ordering on the shelf alone would reorder again and again during the lead time.
- **Demand that waits.** Here an order waits in the `backlog` until a kit is there: an assembly of one kit and one order. With `when_blocked: drop` on the demand, and a limited backlog, orders that find no stock are lost sales instead.
- **Starting stock.** `starts_with` fills the queue at time 0. A warm-up long enough for a few replenishment cycles keeps the start from shaping the results.

## In the results

- The stocked queue reports *empty*, the share of time it was out of stock, and *mean parts*, the average stock.
- The source reports *orders placed* and *on order*.
- The backlog's *waiting time* is how long customers wait for stock: the service you're buying with the stock.

Try the reorder point and order size as [decisions](../scenarios.md): more stock costs space and money, less stock costs waiting.
