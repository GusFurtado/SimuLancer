# Washing machine assembly line

Two models of washing machine, standard (70%) and premium (30%), go down one line of six stations on two weekday shifts (06:00–22:00, Monday to Friday).

```
release → frame → drum → wiring → test → pack → shipping
                                   ↑  ↓ 8%
                                  rework
```

## Modeling assumptions

- **Release:** one machine every 2.5 minutes, as planned; the mix is random, not a production sequence.
- **Buffers:** the stations are linked by accumulating conveyors of 4.6 m, room for 4 machines each. A full conveyor blocks the station before it.
- **Drum** jams about every 4 hours of running (exponential); a jam takes 5 to 30 minutes, usually about 10.
- **Wiring** has two stations side by side; it's the slowest step, especially for the premium model (4.5–5.5 min).
- **Test** fails 8% of machines. They go to rework, which needs the one technician per shift, and come back to test ahead of new machines.
- **Pack** needs a 1.5-minute changeover when the model changes (other packing material).
- **Results** are measured over four weeks after a one-day warm-up, 20 replications. Throughput is per working hour of the two shifts.

## Data

The times are illustrative, not measured. Replace them with your own time studies and failure records.

## Not modeled

- Material supply to the stations: parts are always there.
- Operators other than the rework technician, breaks and absences.
- A machine failing test twice is as likely as the first time.
