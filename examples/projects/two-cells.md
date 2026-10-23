# Two cells and inspection

Two parallel milling cells feed one inspection station. Inspection sends 10% of parts back to cell A for rework.

```
arrivals ─┬─> mill A ─┐                 ┌─> shipping (90%)
          │           ├─> inspect ──────┤
          └─> mill B ─┘                 └─> mill A (rework, 10%)
```

## Modeling assumptions

- **Routing:** each new part goes to the cell whose conveyor holds fewer parts; ties go to cell A.
- **Merging:** inspection takes parts from both cells in the order they arrive.
- **Rework first:** mill A takes reworked parts before new ones.
- **Mill B** breaks down about every hour (5–15 minutes to repair); mill A doesn't.
- **Results** are measured over 120 hours after an 8-hour warm-up, 30 replications.

## Data

Illustrative. The model shows how routing and merging rules change where parts wait.
