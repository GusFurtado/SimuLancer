extends SceneTree
## Headless test: the viewer's replay must reproduce the engine's state.
## Run by tests/run_all.sh with four results folders made by `outliers-sim run`:
##   godot --headless --path viewer -s res://tests/test_replay.gd -- <deterministic> <two-servers> <routing> <kits>

var failures := 0


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 4:
		printerr("usage: -- <deterministic dir> <two-servers dir> <routing dir> <kits dir>")
		quit(2)
		return
	test_deterministic(args[0])
	test_two_servers(args[1])
	test_routing(args[2])
	test_kits(args[3])
	if failures == 0:
		print("test_replay: all checks passed")
	quit(1 if failures > 0 else 0)


func check(condition: bool, what: String) -> void:
	if not condition:
		failures += 1
		printerr("FAIL: ", what)


func check_positions(actual: Array, expected: Array, what: String) -> void:
	var positions := actual.map(func(item: Array) -> float: return item[1])
	var ok := positions.size() == expected.size()
	for i in mini(positions.size(), expected.size()):
		ok = ok and absf(positions[i] - expected[i]) < 1e-6
	check(ok, "%s: positions %s, expected %s" % [what, positions, expected])


func load_replay(dir: String) -> Replay:
	var replay := Replay.new()
	var error := replay.load_file(dir.path_join("events.jsonl"))
	check(error == "", "load %s: %s" % [dir, error])
	return replay


func test_deterministic(dir: String) -> void:
	var r := load_replay(dir)
	check(r.header["model"] == "deterministic", "header names the model")
	check(r.end_time == 3600.0, "end time is the run length")

	r.seek(1.0)  # part 0 is halfway along c_in
	check_positions(r.belt("c_in"), [1.0], "t=1 c_in")
	check(r.server_state("machine") == "idle", "t=1 machine idle")
	check(r.created == 1, "t=1 one part created")

	r.seek(10.0)  # processing from 2 s to 32 s
	check_positions(r.belt("c_in"), [], "t=10 c_in")
	check(r.server_state("machine") == "busy", "t=10 machine busy")
	check(r.server_parts("machine") == [0], "t=10 part 0 in the machine")

	r.seek(33.0)  # on c_out since 32 s
	check_positions(r.belt("c_out"), [1.0], "t=33 c_out")
	check(r.server_state("machine") == "idle", "t=33 machine idle")

	r.seek(35.0)  # exited at 34 s
	check(r.exited == 1 and r.in_system() == 0, "t=35 part 0 exited")

	r.seek(61.5)  # part 1 created at 60 s
	check_positions(r.belt("c_in"), [1.5], "t=61.5 c_in")

	r.seek(1.0)  # seeking backward replays from the start
	check_positions(r.belt("c_in"), [1.0], "back to t=1 c_in")
	check(r.exited == 0 and r.created == 1, "back to t=1 counters")

	r.seek(r.end_time)  # 60 parts, the last exits at 3574 s
	check(r.created == 60 and r.exited == 60, "end: 60 created, 60 exited (got %d, %d)" % [r.created, r.exited])


func test_two_servers(dir: String) -> void:
	var r := load_replay(dir)
	# Find a moment when the slow server is mid-way through a part: just
	# before it takes the next one, everything upstream is accumulated.
	var take := -1.0
	for e: Dictionary in r.events:
		if e["type"] == "started_process" and e["server"] == "slow" and float(e["t"]) > 3000.0:
			take = e["t"]
			break
	check(take > 0.0, "slow server takes a part after 3000 s")

	r.seek(take - 1.0)
	# Capacity 4 (2 m / 0.5 m): head at the end, then one spacing apart.
	check_positions(r.belt("c2"), [2.0, 1.5, 1.0, 0.5], "c2 accumulated")
	check_positions(r.belt("c1"), [2.0, 1.5, 1.0, 0.5], "c1 accumulated")
	check(r.server_state("fast") == "blocked", "fast server blocked")
	check(r.server_state("slow") == "busy", "slow server busy")

	# When the head leaves, the last part is already one spacing in, so the
	# fast server's blocked part gets on at once and everyone moves up.
	r.seek(take + 0.25)
	check_positions(r.belt("c2"), [1.75, 1.25, 0.75, 0.25], "c2 moving up after the head left")
	check(r.server_state("fast") == "busy", "fast server unblocked and took the next part")

	r.seek(take + 1.0)  # closed up again
	check_positions(r.belt("c2"), [2.0, 1.5, 1.0, 0.5], "c2 closed up")


func test_routing(dir: String) -> void:
	var r := load_replay(dir)
	# Every part in the system is somewhere visible: on a conveyor or in a
	# server, except one a blocked source may be holding.
	for t: float in [600.0, 1800.0, 3600.0, 5400.0, r.end_time]:
		r.seek(t)
		var visible := 0
		for c: String in r.conveyors:
			visible += r.belt(c).size()
		for s: String in r.servers:
			visible += r.server_parts(s).size()
		var hidden := r.in_system() - visible
		check(hidden >= 0 and hidden <= r.header["sources"].size(),
			"t=%d: %d in system, %d visible" % [t, r.in_system(), visible])
	var used := {}
	for e: Dictionary in r.events:
		if e["type"] == "entered_conveyor":
			used[e["conveyor"]] = true
	for c: String in ["in_a", "in_b", "rework", "retest", "ship"]:
		check(used.has(c), "some part used conveyor %s" % c)


func test_kits(dir: String) -> void:
	var r := load_replay(dir)
	# Kits, batches and shifts: every part in the system is still somewhere
	# visible, except one each blocked source may be holding.
	var off_seen := false
	for t: float in [1800.0, 3600.0, 23.5 * 3600.0, 24.5 * 3600.0, r.end_time]:
		r.seek(t)
		var visible := 0
		for c: String in r.conveyors:
			visible += r.belt(c).size()
		for s: String in r.servers:
			visible += r.server_parts(s).size()
			off_seen = off_seen or r.server_state(s) == "off"
		var hidden := r.in_system() - visible
		check(hidden >= 0 and hidden <= r.header["sources"].size(),
			"kits t=%d: %d in system, %d visible" % [t, r.in_system(), visible])
	check(off_seen, "the oven was seen off shift")
	check(r.assembled > 0 and r.consumed == 3 * r.assembled, "kits: %d assembled, %d consumed" % [r.assembled, r.consumed])
