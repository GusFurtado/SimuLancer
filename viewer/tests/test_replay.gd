extends SceneTree
## Headless test: the viewer's replay must reproduce the engine's state.
## Run by tests/run_all.sh with two results folders made by `outliers-sim run`:
##   godot --headless --path viewer -s res://tests/test_replay.gd -- <deterministic> <two-servers>

var failures := 0


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 2:
		printerr("usage: -- <deterministic results dir> <two-servers results dir>")
		quit(2)
		return
	test_deterministic(args[0])
	test_two_servers(args[1])
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
