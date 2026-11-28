# The templates

New projects can start from a template, a complete model with its documentation, decisions and results already set up. Each shows a few patterns at work; their files are in [`examples/`](https://github.com/GusFurtado/SimuLancer/tree/main/examples), and each project's README explains how it was modeled and what it leaves out.

| Template | What it shows | Patterns |
|---|---|---|
| **Conveyor line** | A serial line where failures spread as blocking and starving | [Breakdowns](failures.md), accumulating conveyors |
| **Two cells and inspection** | Parallel cells, merging and a rework loop | Routing and merging rules, [rework](rework.md) with priority |
| **Assembly line** | A line of six stations linked by short conveyors, with two models of washing machine | [Shifts](shifts.md), changeovers, a [rework and scrap](rework.md) decision with attributes, a technician as a [resource](shared-resources.md) |
| **Bank branch** | Customers by time of day, lines with priority, giving up | [Priorities and impatience](priorities.md), [shifts](shifts.md) with overtime, service levels |
| **Bike assembly** | Kits of a frame and two wheels, an oven that cures in batches | Assembly, batches, curing carts as [resources parts carry](shared-resources.md) |
| **Distribution warehouse** | Restocked racks, forklifts, pallets broken into cartons, picking | [Restocking](restocking.md), [vehicles](vehicles.md), splitting, assembly of orders |

To start from one, click **New project** on the projects page and pick it. To see one without the web app:

```sh
simulancer validate examples/warehouse.yaml
simulancer run examples/warehouse.yaml
```
