# Why open source

Commercial discrete event simulation is dominated by a handful of tools: Rockwell Arena, Simio, AnyLogic, FlexSim, Siemens Plant Simulation, ProModel, WITNESS and ExtendSim. They're mature and capable. They also share a set of problems that have little to do with simulation and a lot to do with how they're sold, built and used. SimuLancer is a response to those problems.

This page lists them, theme by theme, with what SimuLancer does about each. Not every problem applies to every tool, but most apply to most of them. The table below shows where each tool stands on the facts that can be checked.

## The tools side by side

| | License | Price | Free edition | Runs on | Model files | Logic written in |
|---|---|---|---|---|---|---|
| **Arena** | Proprietary | On request | Student: model size limited, no commercial use | Windows | `.doe`, opened by Arena only | SIMAN expressions, VBA |
| **Simio** | Proprietary | On request | Personal Edition: model size limited | Windows (on a Mac, inside a Windows VM) | Simio project files | Processes, C# for extensions |
| **AnyLogic** | Proprietary | On request | PLE: 10 agent types, 5 hours of model time, 50,000 agents; learning only | Windows, macOS (Linux not guaranteed) | XML (`.alp`, or `.alpx` split into files); Git add-on, not in PLE | Java |
| **FlexSim** | Proprietary (Autodesk) | On request | Express: 30 objects, 35 Process Flow activities | Windows 64-bit | `.fsm`; XML (`.fsx`) optional | FlexScript |
| **Plant Simulation** | Proprietary (Siemens) | On request | Student: 80 objects, no commercial use | Windows | `.spp` | SimTalk; Python through COM |
| **ExtendSim** | Proprietary | Published | Student package (paid), trial | Windows (on a Mac, through Parallels or Boot Camp) | ExtendSim model files | ModL |
| **JaamSim** | Open source (Apache-2.0) | Free | The full program | Windows, macOS, Linux (Java) | Text (`.cfg`) | Expressions; new objects in Java |
| **SimPy** | Open source (MIT) | Free | The full library | Wherever Python runs | Python code | Python |
| **SimuLancer** | Open source (MIT or Apache-2.0) | Free | The full program | Server: Linux or Docker. Users: any browser | YAML text | None: the model is data, plus a small formula language |

Checked on 2026-11-25 against the vendors' own pages where they exist (see [Sources](#sources)). Vendors change editions and limits often: if something here is out of date, [open an issue](https://github.com/GusFurtado/SimuLancer/issues). ProModel and WITNESS aren't in the table yet.

Two things stand out. First, the open-source tools aren't new: JaamSim and SimPy have been around for years, and the problems below aren't solved just by publishing the source. JaamSim is a desktop program for modelers, and SimPy is a library for programmers. SimuLancer aims at something neither does: a model that engineers review like code and managers question in plain language, with the statistics done for them. Second, a few commercial tools have moved: AnyLogic models are XML with Git built in, and FlexSim can save XML. They're the exceptions.

## Where the commercial tools are ahead

Be clear about what you give up:

- **3D.** Most commercial tools animate in 3D. SimuLancer animates in 2D, on purpose (see [section 8](#8-animation-used-as-a-substitute-for-rigor)), but if a client needs a 3D walkthrough, it won't make one.
- **Optimization.** Most commercial tools include an optimizer, often OptQuest. SimuLancer has none: you can sweep parameters through the [API](api.md).
- **Breadth.** Decades of libraries: material handling, pedestrians, fluids, agent-based and system dynamics models. SimuLancer covers the lines, cells, stores and service systems described in [Model files](models.md), and not yet the rest.
- **Maturity and support.** Thousands of users, consultants, courses and support contracts. SimuLancer is in early development, and its model format can still change.

---

## 1. Cost and licensing

- **High prices.** Commercial seats usually cost five figures per user. Professional and enterprise tiers can be well above that.
- **Subscriptions.** Perpetual licenses have mostly been replaced by yearly subscriptions. If you stop paying, you can't open your own models.
- **Add-ons cost extra.** Optimization (e.g. OptQuest), 3D, extra libraries, database connectors, experiment runners and cloud execution are often sold separately.
- **Runtime and distribution fees.** Giving a model to a client so they can run it can require a separate runtime or viewer license.
- **Node-locked or dongle licenses.** Licenses tied to one machine, license servers and hardware keys make remote work, CI and cloud scaling difficult.
- **Pricing is hidden.** You usually can't find list prices without talking to sales, so it's hard to compare tools or budget for them. ExtendSim is the exception in the table.
- **Student versions are crippled.** Free or academic editions limit the number of entities, objects or model size, and often forbid commercial use. Students learn on toy models. Plant Simulation caps even paid tiers: 500 objects in Foundation, 4,000 in X Standard.
- **Using more compute costs more.** Running replications in parallel or in the cloud may need more licenses or a paid cloud tier, so doing statistically sound work costs extra.
- **A powerful workstation for every user.** Each person who models needs a high-end Windows machine, with a fast CPU, lots of RAM and a graphics card for 3D. FlexSim, for instance, asks for 16 GB of RAM at least and recommends 32 GB. The machine sits idle most of the day and is overkill for everything else its user does. Simulation load comes in bursts, but the hardware is bought per seat.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| High prices, subscriptions | Open source (MIT or Apache-2.0). No license fees, ever. | done |
| Add-ons cost extra | Comparison, the report, the animation, the API, the server and parallel runs are all in the one free program. | done |
| Runtime and distribution fees | Anyone can run a model: one binary, a Docker image, or a browser on the company's server. | done |
| Node-locked or dongle licenses | No licenses to lock. Runs headless, in containers and in CI. | done |
| Pricing is hidden | There's no price. | done |
| Crippled student versions | Students get the full program. | done |
| More compute costs more | Replications run on every core at no extra cost. | done |
| A powerful workstation per user | One server runs the simulations for everyone, and people only need a browser. Load comes in bursts, so one shared machine stays busier than many idle workstations. The engine is efficient, so it needn't be a big machine either: 20 replications of four weeks of the bike line take about a tenth of a second, report included. | done |

## 2. Lock-in

- **Closed file formats.** Models are stored in binary or undocumented formats that only the vendor's tool can read.
- **Skills don't transfer.** Each tool has its own scripting language (SIMAN, SimTalk, FlexScript, ModL, Arena expressions, Simio processes), so years of experience don't carry over to another tool.
- **No migration path.** No tool can convert models to another tool or to a general-purpose language. Switching vendors means rebuilding every model from scratch.
- **Sunk-cost trap.** After paying for licenses, training and consultants, organizations keep defending the tool instead of judging it.
- **Consulting ecosystem.** Vendors and their certified partners make money from consulting, so the tool's complexity serves their business model.
- **Vendor survival risk.** Products get acquired, renamed, deprioritized or retired. FlexSim now belongs to Autodesk, and WITNESS has changed owners. Your models depend on the vendor's business decisions.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Closed file formats | Models are plain YAML; results are JSON, CSV and JSON Lines; the formats are documented and versioned. | done |
| Skills don't transfer | No proprietary scripting language: the model is data, plus a small, documented formula language for distributions. | done |
| No migration path | The model is readable text and the engine is open, so it can be converted or reimplemented. Importers from other tools are being considered ([#88](https://github.com/GusFurtado/SimuLancer/issues/88)). | partial |
| Consulting ecosystem | Models are readable enough that an organization can own them, with separate pages for building the model (Modeling) and for deciding with it (Simulation), so nobody depends on an outside expert to operate the tool. | partial |
| Vendor survival risk | Open source: anyone can keep it alive. | done |

## 3. Old architecture

- **Old foundations.** Several tools are built on designs from the 1980s and 1990s. Arena still runs on SIMAN underneath, and you can see it in error messages, limits and odd behavior.
- **Windows-only desktop apps.** Most require Windows, so they don't run natively on Linux, macOS, servers or containers.
- **Single-threaded engines.** The simulation core usually runs on one CPU thread. Parallel runs, if supported at all, are added on top and may need extra licenses.
- **Slow startup and heavy UI.** Large installers, slow model loading and GUIs that slow down as models grow.
- **The GUI is the only full interface.** Running headless or from a script is missing, limited, awkward or a paid add-on.
- **Weak APIs.** Where APIs exist, they are often COM- or .NET-specific, poorly documented and incomplete.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Old foundations, Windows-only | A Rust engine; a Linux binary, a Docker image, and a web app in any browser. | done |
| Single-threaded engines | Replications run in parallel across all cores, with results independent of the thread count. | done |
| Slow startup and heavy UI | 30 replications of most examples take under 0.2 s, report included. The web app is plain HTML and JavaScript with nothing to install, and loads nothing from the internet (not even a font). | done |
| The GUI is the only full interface | The command line, the HTTP API and the web app are equal citizens; the web app uses only the public API. | done |
| Weak APIs | A documented JSON [HTTP API](api.md), scriptable from any language. | done |

## 4. Reproducibility and engineering practice

- **No real version control.** Binary model files can't be diffed or merged, and code review isn't possible. Two people can't work on one model at once.
- **Huge files, multiplied by copies.** Model files are large, often megabytes, because they bundle graphics, imported data and sometimes results with the logic. With no version control, people version by saving copies (`line_v3_final_REVISED`), so every version costs the full size again, folders fill with near-identical files, and nobody is sure which copy is current.
- **Logic is scattered.** Model logic is split across dialog boxes, property panels, small expression fields and visual blocks. Nobody can read the whole model top to bottom.
- **No automated testing.** There's no standard way to write unit tests for model logic or run regression tests when a model changes.
- **Hard to automate.** It's difficult to run models as part of a pipeline (CI, scheduled jobs, parameter sweeps on a cluster).
- **Results depend on the software version.** Random number streams or engine behavior can change between versions, so old results can't be reproduced exactly.
- **Models die when the license expires.** If the license lapses, nobody can audit or rerun the model, which defeats the point of reproducible work.
- **Documentation is written by hand, if at all.** There's no way to produce an accurate, complete specification from the model itself.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| No real version control | Projects are git repositories: on the Modeling page, **Save version** commits with a message about what changed and **Publish** tags it; the Simulation page only shows published versions. | done |
| Huge files, multiplied by copies | A model is a few kilobytes of text (the templates are 1.4 to 2.5 KB), and each saved version is a git commit that stores only what changed: 100 saved versions of the warehouse model take 37 KB once packed, and there's one current file. Run results are kept apart from the project and can be recreated from the model and the seed. Each run can be deleted from *Recent runs*. | done |
| Logic is scattered | The whole model is one readable file; `validate` shows everything the engine understood. | done |
| No automated testing | The engine is [validated against queueing theory](methods/validation.md) on every build; models and scenarios can be checked in CI with `validate`. | done |
| Hard to automate | A headless [command line](cli.md) and the [HTTP API](api.md); a Python parameter sweep is in the API docs. | done |
| Results depend on the software version | Its own random number generator and distributions, so the same seed gives the same results; time is kept in integers. | done |
| Models die with the license | No license to expire. | done |
| Documentation by hand | The model file is the specification; `validate` prints an exact summary. What the model can't say (assumptions, business rules, data sources) goes in the project's `README.md`, written on the Modeling page's *Documentation* tab and versioned with the model; templates come with one. | partial |

## 5. Modeling paradigm

- **Too much hidden in the GUI.** Drag-and-drop blocks hide what the model actually does. Similar-looking blocks can have different queueing, seizing or preemption behavior.
- **Implicit default behavior.** Default queue rules, resource release rules, tie-breaking and event ordering are rarely visible, and they change results.
- **Too many overlapping features.** There are several ways to do the same thing (blocks, processes, add-on processes, scripts, events), each with its own subtleties.
- **Abstraction leaks.** Once logic goes beyond the standard templates, you end up writing code in a weak proprietary language inside small text fields.
- **Weak general-purpose programming.** The built-in languages lack proper data structures, libraries, debuggers, type systems and package ecosystems.
- **Hard to model non-standard systems.** The tools are tuned for factories and warehouses. Anything that doesn't fit the "entities flow through stations" pattern becomes a struggle.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Too much hidden in the GUI | The text file is the source of truth. The web app, drawing included, edits it, so nothing lives only in the GUI. | done |
| Implicit default behavior | Tie-breaking, blocking and failure rules are documented; routing and merging rules are required whenever there's a choice. | done |
| Too many overlapping features | One way to express each thing. | done |
| Abstraction leaks, weak programming | A deliberately small model language, plus the API for anything beyond it; a scripting escape hatch is an open question. | partial |
| Hard to model non-standard systems | Lines, cells, stores and service systems: failures, part types, shared resources, shifts, batches, assembly and splitting, restocking, vehicles, paced lines, attributes, priorities, conditional routing and customers who give up. Several of them are checked against exact results. Still missing: non-accumulating conveyors, and a scripting escape hatch. See [Model files](models.md). | partial |

## 6. Data integration

- **Clumsy data input.** Importing from databases, APIs or data lakes often relies on Excel/CSV bridges, ODBC or paid connectors.
- **Rigid output formats.** Results come as built-in reports or proprietary output files. Getting raw event logs into Python or R for analysis is a chore.
- **Weak input modeling.** Built-in distribution fitters are simple. There's little support for non-stationary arrivals, correlated inputs, empirical/trace-driven inputs or uncertainty in the input parameters themselves.
- **Disconnected from modern data tools.** No native integration with pandas, Arrow, Parquet, SQL warehouses, notebooks or ML libraries.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Clumsy data input | Models are YAML (easy to generate) and scenarios are change sets sent through the API. No database connectors yet. | partial |
| Rigid output formats | A JSON summary, long-format CSV, a JSON Lines event log and a self-contained HTML report. See [Results](results.md). | done |
| Weak input modeling | Seven distributions, including Weibull and gamma, and arrival rates by time of day. No distribution fitting yet. | partial |
| Disconnected from modern data tools | CSV and JSON open in pandas or R directly. No Parquet or Python package yet. | partial |

## 7. Statistics and methodology

- **The easiest thing to do is run once.** Hitting "play" and watching the animation is the easiest workflow, so many studies rely on one replication or too few.
- **Warm-up and run length are guessed.** There's little guidance or automation for spotting the initialization bias and choosing the warm-up period.
- **Confidence intervals are an afterthought.** Output analysis is often buried or optional, and point estimates get presented without their uncertainty.
- **Optimization is a black box.** Built-in optimizers (often licensed third-party engines) are used without understanding of their stopping rules, noise handling or how they treat multiple comparisons.
- **Validation is skipped.** Nothing in the workflow forces a comparison with real-system data, so models get used as soon as they "look right."
- **Verification is hard.** Because logic is hidden and scattered (sections 4 and 5), it's hard to confirm that the model does what the modeler intended.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| The easiest thing is to run once | Replications are the default; every number comes with a 95% confidence interval; the report shows the spread across replications. See [Replications](methods/replications.md). | done |
| Warm-up and run length are guessed | Every report has a [warm-up check](methods/warmup.md) (Welch's method and the MSER rule), and `run` warns when the warm-up may be too short or the system never settles. Run length isn't checked yet. | done |
| Confidence intervals are an afterthought | Every measure, in every output, has its interval. | done |
| Comparing scenarios is unsound | Paired comparison on [common random numbers](methods/comparisons.md): an interval on the difference and a verdict. The Simulation page says it in words: *"Lead time: 9.10 min → 7.24 min, down 20% (between 18% and 23%) · better"*. | done |
| Optimization is a black box | No optimizer yet; parameter sweeps through the API. | open |
| Validation is skipped | The engine itself is [validated against queueing theory](methods/validation.md). Nothing yet helps validate a user's model against real data. | partial |
| Verification is hard | Readable models, a per-part event log, and a replay that reproduces the engine exactly. | done |

## 8. Animation used as a substitute for rigor

- **The animation is the product.** 3D animation sells the tool and the project to decision-makers, but it isn't evidence.
- **It creates false credibility.** A polished 3D factory gets trusted far more than its untested input data deserves.
- **Effort goes into the wrong place.** Hours go into 3D models, textures and camera paths instead of data collection, validation and experimental design.
- **Animation is slow.** Running with animation on is slow, and some workflows push people to keep it on.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| The animation is the product | The Simulation page leads with plain-language results and their ranges; each scenario's report and animation are one click below. | done |
| Effort goes into 3D | Layouts are a small separate file, and the animation is 2D; there's no 3D to sink hours into. | done |
| Animation is slow | Simulations never run with animation: they run at full speed, then the event log is replayed. | done |

## 9. The "pseudo-science" pipeline

- **Results are decided before modeling starts.** Consulting engagements are often paid to justify an investment that has already been decided. A model that's hard to inspect makes that easy.
- **Models can't be audited.** Third parties can't review a model they can't open without a license, read as text or rerun.
- **Assumptions aren't documented.** Key assumptions sit in property fields and are never written up for the people who rely on the results.
- **No peer review.** Unlike academic work, industry simulation studies are almost never reviewed or replicated.
- **The tool lends false authority.** "We simulated it in [expensive software]" is treated as proof, regardless of how good the model is.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Models can't be audited | Anyone can read and rerun a model for free, and get identical numbers. | done |
| Assumptions aren't documented | Every assumption is a line in the model file, each saved version carries a message about what changed, and what can be changed on the Simulation page is spelled out as named decisions with their options and limits. | partial |
| The tool lends false authority | Results always come with their uncertainty. "No clear difference" is a valid answer, and the Simulation page says so in words, with the range, instead of showing a single number. | done |

## 10. Learning curve and training

- **Expensive training.** Official courses and certifications are another revenue stream.
- **Tool training replaces method training.** Courses teach where to click instead of the theory: queueing, probability, statistics and experimental design.
- **Knowledge is concentrated in a few people.** Only one or two people in an organization can maintain a model, so turnover leaves models orphaned.
- **Weak documentation.** Help files are often thin, outdated or locked behind customer portals. Community knowledge is small compared with open-source ecosystems.
- **Hard to debug.** Debuggers and tracing tools are primitive compared with modern IDEs, and error messages are often cryptic.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Tool training replaces method training | These docs explain the [methods](methods/replications.md) (confidence intervals, warm-up, common random numbers) next to the tool, and the Simulation page explains in one paragraph why its comparisons are fair. | partial |
| Weak documentation | The docs are public, live in the repository and are built on every change. Still thin on building models. | partial |
| Hard to debug | Errors point to the exact line or form field, in plain language. | done |

## 11. Academia–industry pipeline

- **Universities train students on specific tools.** Free academic licenses get students used to a tool, and they later push employers to buy it.
- **Textbooks tied to a tool.** Some well-known simulation textbooks are built around a specific commercial tool, which mixes the method up with the product.
- **Research is hard to reproduce.** Academic papers whose models need a commercial license can't be reproduced freely.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Research is hard to reproduce | A free tool, text models and fixed seeds: papers can ship models anyone can rerun. | done |

## 12. Collaboration and deployment

- **Not built for teams.** There's no real multi-user editing, branching or review workflow.
- **Hard to share with stakeholders.** Letting someone outside the modeling team run scenarios needs a license, a viewer, an expensive cloud product or a custom export.
- **Hard to put into production.** Using a model as a decision-support service (an API, a dashboard, a scheduled run) is difficult or requires premium products.
- **Digital twin marketing.** "Digital twin" features are sold heavily, but connecting live data to a simulation is usually much more fragile than the brochures suggest.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Not built for teams | Separate pages for building (Modeling: build, save, publish) and deciding (Simulation: published versions only), plus the API for scripts and other programs. Concurrent editing and roles come later. | partial |
| Hard to share with stakeholders | Anyone with the link opens the Simulation page, builds [scenarios](scenarios.md) from the decisions the model offers (options and bounded numbers, so they can't break the model) and reads comparisons in plain language. Exporting a comparison for a meeting is planned. | partial |
| Hard to put into production | The server and API are the product: a static binary or a Docker image, health checks and [deployment docs](deploy.md). Webhooks are planned. | partial |

## 13. Scale and performance

- **Large models don't scale well.** Big models (many entities, long horizons, detailed transport logic) become slow and use a lot of memory.
- **Big experiments are expensive.** Thousands of replications or large parameter sweeps take a long time, or cost extra licenses or cloud credits.
- **No GPU or distributed engine.** Modern parallel and distributed computing is barely used.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Large models, big experiments | A fast native engine on all cores, with no allocation per event. Not yet tested on very large models. | partial |
| No distributed engine | One machine only, for now. | open |

## 14. Modern workflows and AI

- **Models are hard for AI tools to work with.** Binary, GUI-bound models can't easily be read, generated or edited by code assistants or LLMs, unlike models written as code.
- **No programmatic model building.** Building model structure from data (e.g. creating a model from a process mining log or a facility layout file) is limited or awkward.
- **Weak link to process mining.** Event logs from real systems don't connect cleanly to model building or validation.

**What SimuLancer does**

| Problem | Answer | Status |
|---|---|---|
| Models are hard for AI tools | Plain-text models with clear error messages: easy for code assistants to read, write and fix. SimuLancer itself is built that way. | done |
| No programmatic model building | Models and scenarios can be generated as YAML or sent as JSON through the API. | partial |
| Weak link to process mining | The event log format is close to process-mining logs; no import yet. | open |

## Questions to ask about any DES tool

Use these to judge any tool, open-source or commercial. SimuLancer's answers are on the right.

| Question | SimuLancer |
|---|---|
| Can I read the whole model as plain text and diff it in git? | Yes |
| Can I run it headless from a script, on Linux, in a container? | Yes |
| Can I run thousands of replications in parallel without paying per core? | Yes |
| Can someone without a license audit and rerun the model? | Yes |
| Are queueing, tie-breaking and event-ordering rules explicit and documented? | Yes |
| Does the workflow make replications, warm-up and confidence intervals the default? | Yes |
| Can I get raw event logs out in an open format? | Yes: JSON Lines and CSV (no Parquet yet) |
| Will my models still run in 10 years if the vendor disappears? | The source is open, so anyone can keep building it |
| Can I write automated tests for model logic? | Partly: `validate` in CI, and runs are exactly reproducible from the seed |
| Is the tool helping me do correct analysis, or just a convincing demo? | Judge for yourself: every result carries its interval, and "no clear difference" is a valid answer |

**Status** in the tables above: *done* (built and tested), *partial* (part of it is built), *planned* (designed, not built yet), *open* (not addressed yet). They're kept up to date with every change; last updated 2026-11-25.

## Sources

Checked on 2026-11-25.

- Arena: [system requirements](https://www.rockwellautomation.com/en-ie/products/software/arena-simulation/support/system-requirements.html), [academic editions](https://www.rockwellautomation.com/en-gb/products/software/arena-simulation/academic.html).
- Simio: [installing on a Mac](https://myaccount.simio.com/articles/KB_SoftwareBestPractices/How-to-Install-Simio-on-a-Mac-26-6-2024), [academic program](https://simio.com/academics/order-academic-software).
- AnyLogic: [editions](https://anylogic.help/anylogic/ui/editions.html), [system requirements](https://anylogic.help/anylogic/ui/system-requirements.html), [model formats](https://anylogic.help/anylogic/ui/model-formats.html), [Git add-on](https://anylogic.help/advanced/teamwork/git/about.html).
- FlexSim: [product details and requirements](https://www.autodesk.com/products/flexsim/product-details), [features by license type](https://forums.autodesk.com/t5/flexsim-knowledge-base/flexsim-feature-limitations-by-license-type/ta-p/13530769), [recommended system requirements](https://forums.autodesk.com/t5/flexsim-knowledge-base/recommended-system-requirements/ta-p/13480429), [FlexSim XML and git](https://forums.autodesk.com/t5/flexsim-knowledge-base/flexsim-xml-and-git/ta-p/13585106).
- Plant Simulation: [student download](https://resources.sw.siemens.com/en-US/download-tecnomatix-plant-simulation-student-software), [Plant Simulation X Standard](https://plm.sw.siemens.com/de-DE/tecnomatix/products/plant-simulation-x-standard), [Plant Simulation Foundation](https://www.simplan.de/en/plant-simulation-foundation-license/).
- ExtendSim: [pricing](https://extendsim.com/products/purchase/pricing), [running on a Mac](https://extendsim.com/faq-general/858-faqgeneralmac), [academic](https://extendsim.com/academic).
- JaamSim: [source and license](https://github.com/jaamsim/jaamsim).
- SimPy: [documentation](https://simpy.readthedocs.io).
