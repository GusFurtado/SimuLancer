# Bike assembly

Frames and wheels arrive during two weekday shifts (06:00–22:00). Fitters assemble one frame and two wheels into a bike, and bikes cure in an oven, eight at a time, before shipping.

```
frames ─┐
        ├─> assembly ──> oven ──> shipping
wheels ─┘   (fitters)   (8 at a time)
```

## Modeling assumptions

- **Arrivals** are random: a frame about every 3 minutes, a wheel every 1.5 minutes, so kits are usually complete.
- **Assembly** has four stations but only two fitters per shift; fitters are the limit. A bike takes 4 to 7 minutes, usually 5.
- **Oven** waits for eight bikes and cures them together for 20 minutes.
- **Curing carts:** each bike is built on a cart and stays on it until curing ends, when the cart goes back to assembly empty. There are 16 carts, enough for two oven loads; with fewer, fitters wait for an empty cart and fewer bikes are made.
- **Results** are measured over four weeks after a one-week warm-up (the first week is still filling up), 300 replications: the share of kits waiting for a fitter or a cart needs many to be known within a point. Throughput is per working hour of the two shifts.

## Data

All times are illustrative.

## Not modeled

- Supplier deliveries in lots: frames and wheels come one by one.
- An oven that starts with fewer than eight bikes at the end of a shift.
