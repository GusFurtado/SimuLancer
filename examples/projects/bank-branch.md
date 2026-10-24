# Bank branch

Customers come in on weekdays while the doors are open (10:00–16:00), take a ticket at the kiosk, and wait in line for a teller (quick transactions, 85%) or an account advisor (longer appointments, 15%).

```
                          ┌─ teller line ──> tellers ──┐
door → entrance → kiosk ──┤                            ├──> exit
                          └─ advisor line ─> advisors ─┘
```

## Modeling assumptions

- **Arrivals** are random and busier at lunchtime: 40 an hour from 10:00 to 12:00, 70 from 12:00 to 14:00, 45 from 14:00 to 16:00.
- **Leaving:** when the line at the kiosk is full (5 people), newcomers turn around and leave. They are counted as lost customers.
- **Lines** are first come, first served, except that **priority customers** (seniors, pregnant women, people with disabilities; 10% of all customers) go ahead of the others in the teller line. Nobody is interrupted for them. 15 places for tellers, 7 for advisors. The service level is the share who waited at most 5 minutes for a teller, 15 for an advisor.
- **Giving up:** customers waiting for a teller leave after 20 minutes on average (each has their own patience, exponential). Those waiting for an advisor stay: they came for an appointment.
- **Staff:** 3 tellers for 5 counters and 2 advisors for 3 desks. Staff start at 09:45 and, after closing, serve everyone still inside before going home.
- **Service times** vary a lot (lognormal): about 3 minutes at a teller, 12 with an advisor.
- **Results** cover four weeks, 20 replications.

## Data

The arrival rates and service times are illustrative. Ticket logs from the kiosk are the best source for real ones.

## Not modeled

- Appointments booked in advance.
- Breaks and lunch shifts of the staff.
