# Vehicles

When parts are carried, not conveyed, the carriers can be the bottleneck: a forklift fleet that's busy driving empty, or AGVs queuing at a busy station. **Vehicles** carry parts along **transports**.

```yaml
format: simulancer/0.1
name: forklift-shuttle
entities:
  pallet: {}
sources:
  dock: { creates: pallet, interarrival: "exponential(3 min)" }
servers:
  unload: { capacity: 2, process_time: "uniform(1 min, 2 min)" }
  store: { process_time: "30 s" }
vehicles:
  forklifts: { count: 2, speed: "2 m/s", load_time: "20 s", unload_time: "20 s", home: unload }
queues:
  arriving: { from: dock, to: unload }
  stored: { from: store, to: racks }
transports:
  to_store: { from: unload, to: store, by: forklifts, distance: "80 m" }
sinks:
  racks: {}
experiment:
  replications: 20
  warmup: "1 d"
  run_length: "14 d"
  seed: 1
```

## Choices

- **A transport is a connection**, like a conveyor or a queue, but a vehicle carries each part. A part ready to leave waits where it is; the nearest idle vehicle drives there empty, loads it, drives the `distance`, unloads it, and stays until it's needed again.
- **Distance, not a map.** Each transport has its distance. Empty trips take as long as a transport between the two places; without one, as long as the transport being fetched for. That's enough for a fleet's utilization and waiting times, not for traffic between vehicles.
- **One part per trip.** Each trip carries one part. For a tugger that pulls a train of carts, assemble the carts into one train part before the transport (`assembles`, see [Batches and assembly](../models.md#batches-and-assembly)) and split it back after (`splits_into`).
- **`home`** is where vehicles start. They don't go back to it between trips.
- Vehicles work around the clock; they have no schedule yet.

## In the results

- The transport reports *waiting for pickup*: from the moment a part is ready to the moment a vehicle gets there. It's the cost of too few vehicles.
- The fleet reports *busy* (driving, loading or unloading), *empty travel* (the part of it spent driving empty) and *trips*. A fleet busy 85% with a third of it empty travel is a candidate for better dispatching or a closer home, not only more vehicles.
- Try the fleet size as a [decision](../scenarios.md).
