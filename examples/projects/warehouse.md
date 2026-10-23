# Distribution warehouse

Trucks bring pallets, which forklifts put away in the racks. Pallets are broken into cartons on the pick shelves. Customer orders are picked, packed and dispatched.

```
trucks → dock → unload ═> putaway → racks → break pallet → shelves ─┐
                       (forklifts)                                  ├─> pick → pack → dispatch
orders ──────────────────────── backlog ────────────────────────────┘
```

## Modeling assumptions

- **Restocking:** when the pallets in the racks, with those already ordered, fall to 250, a truck of 25 pallets is ordered. It comes 18 to 30 hours later, on the morning shift only (06:00–14:00, weekdays).
- **Receiving:** two dock doors, 1–2 minutes to unload a pallet. Three forklifts carry pallets 60 m to the racks, one at a time, and drive back empty.
- **Racks** hold 300 pallets (150 at the start); the **pick shelves** 400 cartons (200 at the start). A pallet is broken into 10 cartons only when the shelves have room, so the shelves restock themselves.
- **Orders** come in about every 90 seconds during operations (06:00–22:00, weekdays). Every order is **two cartons**.
- **Picking** needs one of 5 pickers; a shipment's time in the warehouse counts from when its order came in.
- **Results** are measured over four weeks after a one-week warm-up, 20 replications.

## Data

Order rates, pallet sizes and times are illustrative.

## Not modeled

- Orders of different sizes, and stock-outs of particular products: all cartons are the same.
- Forklift schedules and paths between rack aisles (a fixed 60 m trip).
- Dispatch by carrier: shipments leave as soon as they're packed.
