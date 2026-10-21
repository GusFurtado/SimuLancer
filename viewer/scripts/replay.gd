class_name Replay
extends RefCounted
## Rebuilds the state of a simulation at any moment from its event log.
##
## Conveyors follow the engine's rules exactly (DECISIONS.md, "Engine core"):
## parts are points moving at the belt speed, the head stops at the end,
## followers stop one spacing behind the part ahead, and everyone moves again
## when the head leaves. Positions are computed on demand, like in the engine.

const FORMAT := "outliers-sim-events/0.1"

var header: Dictionary = {}
var events: Array[Dictionary] = []
var warmup: float = 0.0
var end_time: float = 0.0

## Current playback time, in seconds.
var time: float = 0.0
## name -> {from, to, length, speed, spacing, capacity, belt: [{part, pos, since}]}
var conveyors: Dictionary = {}
## name -> {capacity, busy: [part], blocked: [part], failed: failure name or "", off: off shift}
var servers: Dictionary = {}
## name -> parts exited
var sinks: Dictionary = {}
var created: int = 0
var lost: int = 0
var exited: int = 0
## Parts made by assembly servers, components they used up (and parts
## split into others), and parts made by splitting.
var assembled: int = 0
var consumed: int = 0
var split: int = 0

var _cursor: int = 0


## Reads an events.jsonl (or gzipped events.jsonl.gz) file. Returns an error
## message, or "" on success.
func load_file(path: String) -> String:
	if not FileAccess.file_exists(path):
		return "can't open %s: file not found" % path
	var bytes := FileAccess.get_file_as_bytes(path)
	if path.ends_with(".gz"):
		bytes = bytes.decompress_dynamic(-1, FileAccess.COMPRESSION_GZIP)
		if bytes.is_empty():
			return "can't decompress %s" % path
	var first := true
	for line in bytes.get_string_from_utf8().split("\n", false):
		var parsed: Variant = JSON.parse_string(line)
		if typeof(parsed) != TYPE_DICTIONARY:
			return "%s: not a JSON object: %s" % [path, line.left(80)]
		if first:
			header = parsed
			first = false
		else:
			events.append(parsed)
	if header.get("format", "") != FORMAT:
		return "%s is not an outliers-sim event log (expected format %s)" % [path, FORMAT]
	warmup = header["warmup"]
	end_time = header["end"]
	reset()
	return ""


## Back to the start of the run.
func reset() -> void:
	conveyors.clear()
	for c: Dictionary in header["conveyors"]:
		conveyors[c["name"]] = {
			"from": c["from"], "to": c["to"],
			"length": float(c["length"]), "speed": float(c["speed"]),
			"spacing": float(c["spacing"]), "capacity": int(c["capacity"]),
			"belt": [],
		}
	servers.clear()
	for s: Dictionary in header["servers"]:
		# waiting: parts collected into a batch or kit, not yet processing.
		servers[s["name"]] = {"capacity": int(s["capacity"]), "busy": [], "blocked": [], "waiting": [], "failed": "", "off": false}
	sinks.clear()
	for name: String in header["sinks"]:
		sinks[name] = 0
	created = 0
	lost = 0
	exited = 0
	assembled = 0
	consumed = 0
	split = 0
	time = 0.0
	_cursor = 0


## Moves to time `t` (seconds), forward or backward.
func seek(t: float) -> void:
	t = clampf(t, 0.0, end_time)
	if t < time:
		reset()
	while _cursor < events.size() and float(events[_cursor]["t"]) <= t:
		_apply(events[_cursor])
		_cursor += 1
	time = t


## Parts on a conveyor now, head first: [[part id, meters from the entrance], ...].
func belt(conveyor: String) -> Array:
	var c: Dictionary = conveyors[conveyor]
	var out := []
	var positions := _positions(c, time)
	for i in positions.size():
		out.append([c["belt"][i]["part"], positions[i]])
	return out


## "off", "failed", "blocked", "busy" or "idle", in that order of precedence.
func server_state(server: String) -> String:
	var s: Dictionary = servers[server]
	if s["off"]:
		return "off"
	if s["failed"] != "":
		return "failed"
	if not s["blocked"].is_empty():
		return "blocked"
	if not s["busy"].is_empty():
		return "busy"
	return "idle"


## Parts inside a server now (processing, blocked, then collected).
func server_parts(server: String) -> Array:
	var s: Dictionary = servers[server]
	return s["busy"] + s["blocked"] + s["waiting"]


func in_system() -> int:
	return created + assembled + split - consumed - lost - exited


func _apply(e: Dictionary) -> void:
	var t: float = e["t"]
	match e["type"]:
		"created":
			created += 1
		"lost":
			lost += 1
		"entered_conveyor":
			var part := int(e["part"])
			var c: Dictionary = conveyors[e["conveyor"]]
			if servers.has(c["from"]):
				servers[c["from"]]["blocked"].erase(part)
			c["belt"].append({"part": part, "pos": 0.0, "since": t})
		"reached_end":
			pass  # positions already stop at the end
		"started_process":
			var part := int(e["part"])
			var s: Dictionary = servers[e["server"]]
			if s["waiting"].has(part):
				s["waiting"].erase(part)
			else:
				_leave_conveyor(part, e["server"], t)
			s["busy"].append(part)
		"collected":
			var part := int(e["part"])
			_leave_conveyor(part, e["server"], t)
			servers[e["server"]]["waiting"].append(part)
		"consumed":
			# A kit's component (collected), or a finished part that was split.
			servers[e["server"]]["waiting"].erase(int(e["part"]))
			servers[e["server"]]["blocked"].erase(int(e["part"]))
			consumed += 1
		"stocked":
			# In a queue when the run starts.
			created += 1
			conveyors[e["conveyor"]]["belt"].append({"part": int(e["part"]), "pos": 0.0, "since": t})
		"split":
			split += 1
			servers[e["server"]]["blocked"].append(int(e["part"]))
		"assembled":
			assembled += 1
			servers[e["server"]]["waiting"].append(int(e["part"]))
		"finished_process":
			var part := int(e["part"])
			var s: Dictionary = servers[e["server"]]
			s["busy"].erase(part)
			s["blocked"].append(part)
		"failed":
			servers[e["server"]]["failed"] = e["failure"]
		"repaired":
			servers[e["server"]]["failed"] = ""
		"shift_ended":
			servers[e["server"]]["off"] = true
		"shift_started":
			servers[e["server"]]["off"] = false
		"exited":
			_leave_conveyor(int(e["part"]), e["sink"], t)
			sinks[e["sink"]] += 1
			exited += 1
		_:
			push_warning("unknown event type: %s" % e["type"])


## The head part of the conveyor feeding `destination` leaves at `t`.
func _leave_conveyor(part: int, destination: String, t: float) -> void:
	for c: Dictionary in conveyors.values():
		var items: Array = c["belt"]
		if c["to"] == destination and not items.is_empty() and items[0]["part"] == part:
			# Everyone stopped behind the head starts moving again: record
			# real positions as the new baseline, as the engine does.
			var positions := _positions(c, t)
			for i in items.size():
				items[i]["pos"] = positions[i]
				items[i]["since"] = t
			items.pop_front()
			return
	push_error("part %d left for %s but isn't at the head of a conveyor feeding it" % [part, destination])


func _positions(c: Dictionary, t: float) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	var limit: float = c["length"]
	for item: Dictionary in c["belt"]:
		var free: float = item["pos"] + c["speed"] * (t - item["since"])
		var pos := minf(free, limit)
		out.append(pos)
		limit = pos - c["spacing"]
	return out
