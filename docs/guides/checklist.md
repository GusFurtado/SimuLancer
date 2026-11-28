# Modeling checklist

Before a model's results go into a decision, go through these. Each takes minutes, and each catches mistakes that look like insights.

## The question

- [ ] **The decision is written down**, with the options and the results that decide it: "a second packer or a bigger buffer, judged on lead time and staff cost". The project's results (the Simulation page's) are those results.
- [ ] **What's left out is written down** in the project's documentation: what the model doesn't include, and why that doesn't change the answer.

## Verification: does the model do what you meant?

- [ ] **`simulancer validate`** prints what the engine understood: every distribution with its mean, every route, every schedule. Read it once, line by line.
- [ ] **Watch the animation** for a simulated day. Parts that skip a station, queues that never form, a machine that's never blocked: each is a question to answer.
- [ ] **Extreme cases behave:** arrivals twice as fast should fill the queues; a machine that never fails should never be *failed*; one more operator shouldn't make anything slower.
- [ ] **Counts add up:** what enters leaves, is still inside, or is scrapped.

## Validation: does it match the real system?

- [ ] **The people who run the system** have seen the animation and the report and agree it behaves like the real thing.
- [ ] **Key results match history:** throughput, lead time and utilization against the same weeks of real data, measured the same way.
- [ ] **The inputs have a source:** each distribution comes from data or from someone who knows, and the documentation says which. See [Choosing distributions](distributions.md).

## The experiment

- [ ] **Warm-up:** the report's [warm-up check](../methods/warmup.md) doesn't warn.
- [ ] **Run length:** at least a few cycles of everything slow: weeks with shifts, several restocking orders, many failures of each machine.
- [ ] **Replications:** the [replications check](../methods/replications.md) doesn't warn, so the key results are as precise as the model asks.
- [ ] **Comparisons on common random numbers**, judged by the interval on the difference, not by two means side by side. See [Comparing scenarios](../methods/comparisons.md).

## The answer

- [ ] **Robust to the uncertain inputs:** the best option stays best when the inputs nobody is sure about are pushed up and down.
- [ ] **Stated with its uncertainty:** "lead time down 20% (between 18% and 23%)", not "down 20%".
