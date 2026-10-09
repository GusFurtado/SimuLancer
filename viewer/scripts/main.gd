extends Node3D
## outliers-sim viewer: replays events.jsonl on the layout in 3D.
##
## Open a results folder (with events.jsonl and, optionally, layout.json):
##   godot --path viewer -- results/conveyor-line
## Without an argument it opens the bundled demo (res://demo), or asks for a
## file if there is none. Without a layout.json, objects are placed in a row.

const DEMO := "res://demo"

const SPEEDS := [1, 10, 60, 300, 1800, 3600]
const DEFAULT_SPEED := 2  # index into SPEEDS: 60×

# Same colors as the HTML report (dataviz reference palette).
const STATE_COLORS := {
	"busy": Color("#2a78d6"),
	"blocked": Color("#eb6834"),
	"failed": Color("#1baf7a"),
	"idle": Color("#d6d5ce"),
}
const STATE_LABELS := {
	"busy": "Busy",
	"blocked": "Blocked (output full)",
	"failed": "Failed",
	"idle": "Idle (starved)",
}
const PART_COLOR := Color("#f0c674")
const BELT_COLOR := Color("#5c5b57")
const FLOOR_COLOR := Color("#2a2a28")
const OBJECT_COLOR := Color("#8a8984")

const OBJECT_SIZE := 1.2
const BELT_WIDTH := 0.6
const BELT_TOP := 0.3
const PART_SIZE := 0.32

var replay := Replay.new()
var playing := true
var speed_index := DEFAULT_SPEED

var _camera: OrbitCamera
var _positions: Dictionary = {}    # object name -> Vector3
var _paths: Dictionary = {}        # conveyor name -> {points: Array[Vector3], cumulative: Array[float], scale: float}
var _server_materials: Dictionary = {}  # server name -> StandardMaterial3D
var _server_labels: Dictionary = {}     # server name -> Label3D
var _part_nodes: Array[MeshInstance3D] = []
var _part_mesh: BoxMesh
var _part_material: StandardMaterial3D

# HUD
var _clock: Label
var _phase: Label
var _counts: Label
var _play_button: Button
var _speed_button: OptionButton
var _timeline: HSlider
var _message: Label


func _ready() -> void:
	_build_environment()
	_build_hud()
	var args := OS.get_cmdline_user_args()
	if not args.is_empty():
		_open(args[0])
	elif DirAccess.dir_exists_absolute(DEMO):
		_open(DEMO)
	else:
		_ask_for_file()


func _process(delta: float) -> void:
	if replay.header.is_empty():
		return
	if playing:
		replay.seek(replay.time + delta * SPEEDS[speed_index])
		if replay.time >= replay.end_time:
			playing = false
	_refresh()


func _unhandled_key_input(event: InputEvent) -> void:
	if event.is_pressed() and event is InputEventKey:
		match event.keycode:
			KEY_SPACE:
				_toggle_play()
			KEY_PERIOD, KEY_EQUAL, KEY_PLUS:
				_set_speed(mini(speed_index + 1, SPEEDS.size() - 1))
			KEY_COMMA, KEY_MINUS:
				_set_speed(maxi(speed_index - 1, 0))
			KEY_HOME:
				replay.seek(0.0)


# --- loading ----------------------------------------------------------------

func _ask_for_file() -> void:
	_show_message("Open the events.jsonl written by `outliers-sim run`.")
	var dialog := FileDialog.new()
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.filters = PackedStringArray(["*.jsonl, *.jsonl.gz ; outliers-sim event log"])
	dialog.title = "Open event log"
	dialog.use_native_dialog = true
	dialog.file_selected.connect(_open)
	add_child(dialog)
	dialog.popup_centered_ratio(0.7)


## `path` is a results folder or an events.jsonl file.
func _open(path: String) -> void:
	# `godot --path viewer` changes the working directory to the project, so
	# resolve relative paths against the directory Godot was started from.
	var launched_from := OS.get_environment("PWD")
	if path.is_relative_path() and launched_from != "":
		path = launched_from.path_join(path).simplify_path()
	var events_path := path
	if DirAccess.dir_exists_absolute(path):
		events_path = path.path_join("events.jsonl")
		if not FileAccess.file_exists(events_path):
			events_path += ".gz"
	var error := replay.load_file(events_path)
	if error != "":
		_fail(error)
		return
	var layout_path := events_path.get_base_dir().path_join("layout.json")
	if FileAccess.file_exists(layout_path):
		error = _load_layout(layout_path)
		if error != "":
			_fail(error)
			return
	else:
		_auto_layout()
	_build_model()
	_timeline.max_value = replay.end_time
	_show_message("")
	get_window().title = "%s · outliers-sim viewer" % replay.header["model"]


func _load_layout(path: String) -> String:
	var text := FileAccess.get_file_as_string(path)
	var layout: Variant = JSON.parse_string(text)
	if typeof(layout) != TYPE_DICTIONARY or layout.get("format", "") != "outliers-sim-layout/0.1":
		return "%s is not an outliers-sim layout" % path
	for name: String in layout["objects"]:
		var p: Array = layout["objects"][name]["position"]
		_positions[name] = Vector3(p[0], p[1], p[2])
	for c: Dictionary in replay.header["conveyors"]:
		if not layout["conveyors"].has(c["name"]):
			return "%s has no path for conveyor %s" % [path, c["name"]]
		var points: Array[Vector3] = []
		for p: Array in layout["conveyors"][c["name"]]["path"]:
			points.append(Vector3(p[0], p[1], p[2]))
		_paths[c["name"]] = _make_path(points, float(c["length"]))
	return ""


## Places objects in columns by distance from the sources, centered in each
## column. Conveyors run straight between objects; conveyors going backward
## (like rework loops) and self-loops go around the top so they stay visible.
func _auto_layout() -> void:
	var conveyors: Array = replay.header["conveyors"]
	var depth: Dictionary = {}
	var queue: Array[String] = []
	for source: String in replay.header["sources"]:
		depth[source] = 0
		queue.append(source)
	while not queue.is_empty():
		var node: String = queue.pop_front()
		for c: Dictionary in conveyors:
			if c["from"] == node and not depth.has(c["to"]):
				depth[c["to"]] = depth[node] + 1
				queue.append(c["to"])

	var names: Array = replay.header["sources"].duplicate()
	for s: Dictionary in replay.header["servers"]:
		names.append(s["name"])
	names.append_array(replay.header["sinks"])
	var columns: Dictionary = {}  # depth -> names, in header order
	for name: String in names:
		var d: int = depth.get(name, 0)
		if not columns.has(d):
			columns[d] = []
		columns[d].append(name)
	for d: int in columns:
		var column: Array = columns[d]
		for i in column.size():
			_positions[column[i]] = Vector3(d * 10.0, 0.0, (i - (column.size() - 1) / 2.0) * 6.0)

	var top := 0.0
	for p: Vector3 in _positions.values():
		top = minf(top, p.z)
	var loops := 0
	var gap := OBJECT_SIZE / 2.0 + 0.2
	for c: Dictionary in conveyors:
		var a: Vector3 = _positions[c["from"]]
		var b: Vector3 = _positions[c["to"]]
		var points: Array[Vector3] = []
		if depth.get(c["to"], 0) > depth.get(c["from"], 0):
			var direction := (b - a).normalized()
			points = [a + direction * gap, b - direction * gap]
		else:
			# Backward or self-loop: up from the start, across above
			# everything, down into the end. Each loop gets its own lane.
			loops += 1
			var lane := top - 2.0 - loops * 1.5
			var out := a + Vector3(gap * 0.5, 0, -gap)
			var into := b + Vector3(-gap * 0.5, 0, -gap)
			points = [out, Vector3(out.x, 0, lane), Vector3(into.x, 0, lane), into]
		_paths[c["name"]] = _make_path(points, float(c["length"]))


func _make_path(raw_points: Array[Vector3], model_length: float) -> Dictionary:
	# Drop repeated points: zero-length segments can't be drawn or walked.
	var points: Array[Vector3] = [raw_points[0]]
	for p in raw_points.slice(1):
		if not p.is_equal_approx(points[-1]):
			points.append(p)
	var cumulative: Array[float] = [0.0]
	for i in range(1, points.size()):
		cumulative.append(cumulative[-1] + points[i - 1].distance_to(points[i]))
	# Stretch the animation if the drawn path differs from the model's length.
	return {"points": points, "cumulative": cumulative, "scale": cumulative[-1] / model_length}


## Where a part `meters` from the entrance (model length) is drawn.
func _point_on(path: Dictionary, meters: float) -> Vector3:
	var d: float = meters * path["scale"]
	var points: Array[Vector3] = path["points"]
	var cumulative: Array[float] = path["cumulative"]
	for i in range(1, points.size()):
		if d <= cumulative[i] or i == points.size() - 1:
			var seg := cumulative[i] - cumulative[i - 1]
			var f := clampf((d - cumulative[i - 1]) / seg, 0.0, 1.0)
			return points[i - 1].lerp(points[i], f)
	return points[-1]


# --- scene ------------------------------------------------------------------

func _build_environment() -> void:
	var env := WorldEnvironment.new()
	env.environment = Environment.new()
	env.environment.background_mode = Environment.BG_COLOR
	env.environment.background_color = Color("#141413")
	env.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	env.environment.ambient_light_color = Color(0.55, 0.55, 0.55)
	add_child(env)

	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-55, -35, 0)
	sun.light_energy = 1.1
	sun.shadow_enabled = true
	add_child(sun)

	_camera = OrbitCamera.new()
	add_child(_camera)

	_part_mesh = BoxMesh.new()
	_part_mesh.size = Vector3.ONE * PART_SIZE
	_part_material = _material(PART_COLOR)


func _build_model() -> void:
	var bounds := AABB(_positions.values()[0], Vector3.ZERO)
	for p: Vector3 in _positions.values():
		bounds = bounds.expand(p)
	for path: Dictionary in _paths.values():
		for p: Vector3 in path["points"]:
			bounds = bounds.expand(p)

	var ground := MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(bounds.size.x + 20.0, bounds.size.z + 20.0)
	ground.mesh = plane
	ground.position = Vector3(bounds.get_center().x, 0.0, bounds.get_center().z)
	ground.material_override = _material(FLOOR_COLOR)
	add_child(ground)

	for name: String in replay.header["sources"]:
		_add_object(name, Vector3(OBJECT_SIZE, 0.25, OBJECT_SIZE), _material(OBJECT_COLOR), "source")
	for name: String in replay.header["sinks"]:
		_add_object(name, Vector3(OBJECT_SIZE, 0.25, OBJECT_SIZE), _material(OBJECT_COLOR), "sink")
	for s: Dictionary in replay.header["servers"]:
		var material := _material(STATE_COLORS["idle"])
		_server_materials[s["name"]] = material
		_server_labels[s["name"]] = _add_object(s["name"], Vector3(OBJECT_SIZE, 1.0, OBJECT_SIZE), material, "")
	for name: String in _paths:
		_add_belt(_paths[name])

	_camera.frame(bounds.get_center(), maxf(bounds.size.x, bounds.size.z))


func _add_object(name: String, size: Vector3, material: Material, kind: String) -> Label3D:
	var body := MeshInstance3D.new()
	var box := BoxMesh.new()
	box.size = size
	body.mesh = box
	body.material_override = material
	body.position = _positions[name] + Vector3(0, size.y / 2.0, 0)
	add_child(body)

	var label := Label3D.new()
	label.text = name if kind == "" else "%s\n%s" % [name, kind]
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.font_size = 40
	label.pixel_size = 0.006
	label.outline_size = 10
	label.position = _positions[name] + Vector3(0, size.y + 0.9, 0)
	add_child(label)
	return label


func _add_belt(path: Dictionary) -> void:
	var points: Array[Vector3] = path["points"]
	var material := _material(BELT_COLOR)
	for i in range(1, points.size()):
		var a := points[i - 1]
		var b := points[i]
		var belt := MeshInstance3D.new()
		var box := BoxMesh.new()
		box.size = Vector3(BELT_WIDTH, 0.1, a.distance_to(b) + BELT_WIDTH * 0.5)
		belt.mesh = box
		belt.material_override = material
		add_child(belt)
		belt.look_at_from_position((a + b) / 2.0 + Vector3(0, BELT_TOP - 0.05, 0), b + Vector3(0, BELT_TOP - 0.05, 0), Vector3.UP)


func _material(color: Color) -> StandardMaterial3D:
	var m := StandardMaterial3D.new()
	m.albedo_color = color
	m.roughness = 0.85
	return m


## Updates parts, server colors and the HUD for the current replay time.
func _refresh() -> void:
	var placed := 0
	for name: String in _paths:
		for item: Array in replay.belt(name):
			_place_part(placed, _point_on(_paths[name], item[1]) + Vector3(0, BELT_TOP + PART_SIZE / 2.0, 0))
			placed += 1
	for name: String in replay.servers:
		var state := replay.server_state(name)
		(_server_materials[name] as StandardMaterial3D).albedo_color = STATE_COLORS[state]
		var label: Label3D = _server_labels[name]
		label.text = "%s\n%s" % [name, state if state != "failed" else "failed (%s)" % replay.servers[name]["failed"]]
		var parts := replay.server_parts(name)
		for i in parts.size():
			_place_part(placed, _positions[name] + Vector3(0, 1.0 + PART_SIZE * (i + 0.5), 0))
			placed += 1
	for i in range(placed, _part_nodes.size()):
		_part_nodes[i].visible = false
	_update_hud()


func _place_part(index: int, at: Vector3) -> void:
	while _part_nodes.size() <= index:
		var node := MeshInstance3D.new()
		node.mesh = _part_mesh
		node.material_override = _part_material
		add_child(node)
		_part_nodes.append(node)
	_part_nodes[index].position = at
	_part_nodes[index].visible = true


# --- HUD --------------------------------------------------------------------

func _build_hud() -> void:
	var layer := CanvasLayer.new()
	add_child(layer)

	var top := PanelContainer.new()
	top.position = Vector2(16, 16)
	layer.add_child(top)
	var info := VBoxContainer.new()
	top.add_child(info)
	_clock = Label.new()
	_clock.add_theme_font_size_override("font_size", 28)
	info.add_child(_clock)
	_phase = Label.new()
	info.add_child(_phase)
	_counts = Label.new()
	info.add_child(_counts)

	var legend := PanelContainer.new()
	legend.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	legend.position = Vector2(-16, 16)
	legend.grow_horizontal = Control.GROW_DIRECTION_BEGIN
	layer.add_child(legend)
	var rows := VBoxContainer.new()
	legend.add_child(rows)
	for state: String in ["busy", "blocked", "failed", "idle"]:
		var row := HBoxContainer.new()
		var swatch := ColorRect.new()
		swatch.color = STATE_COLORS[state]
		swatch.custom_minimum_size = Vector2(14, 14)
		swatch.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		row.add_child(swatch)
		var text := Label.new()
		text.text = STATE_LABELS[state]
		row.add_child(text)
		rows.add_child(row)

	var bar := PanelContainer.new()
	bar.set_anchors_preset(Control.PRESET_BOTTOM_WIDE)
	bar.offset_left = 16
	bar.offset_right = -16
	bar.offset_top = -72
	bar.offset_bottom = -16
	layer.add_child(bar)
	var controls := HBoxContainer.new()
	controls.add_theme_constant_override("separation", 12)
	bar.add_child(controls)

	_play_button = Button.new()
	_play_button.custom_minimum_size = Vector2(96, 48)
	_play_button.pressed.connect(_toggle_play)
	controls.add_child(_play_button)

	_speed_button = OptionButton.new()
	_speed_button.custom_minimum_size = Vector2(110, 48)
	for s: int in SPEEDS:
		_speed_button.add_item("%d×" % s)
	_speed_button.select(speed_index)
	_speed_button.item_selected.connect(_set_speed)
	controls.add_child(_speed_button)

	_timeline = HSlider.new()
	_timeline.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_timeline.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_timeline.custom_minimum_size = Vector2(200, 32)
	_timeline.step = 0.0
	_timeline.value_changed.connect(func(v: float) -> void: replay.seek(v))
	controls.add_child(_timeline)

	_message = Label.new()
	_message.set_anchors_preset(Control.PRESET_CENTER)
	_message.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_message.grow_horizontal = Control.GROW_DIRECTION_BOTH
	_message.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_message.custom_minimum_size = Vector2(600, 0)
	layer.add_child(_message)


func _update_hud() -> void:
	_play_button.text = "Pause" if playing else "Play"
	_timeline.set_value_no_signal(replay.time)
	_clock.text = _format_clock(replay.time)
	if replay.time < replay.warmup:
		_phase.text = "warm-up (statistics start at %s)" % _format_clock(replay.warmup)
	else:
		_phase.text = "measuring · replication %d" % int(replay.header["replication"])
	_counts.text = "%d created · %d in system · %d exited" % [replay.created, replay.in_system(), replay.exited] + \
		(" · %d lost" % replay.lost if replay.lost > 0 else "")


func _format_clock(seconds: float) -> String:
	var s := int(seconds)
	return "day %d  %02d:%02d:%02d" % [s / 86400 + 1, (s % 86400) / 3600, (s % 3600) / 60, s % 60]


func _toggle_play() -> void:
	if replay.time >= replay.end_time:
		replay.seek(0.0)
	playing = not playing


func _set_speed(index: int) -> void:
	speed_index = index
	_speed_button.select(index)


func _fail(error: String) -> void:
	_show_message(error)
	push_error(error)


func _show_message(text: String) -> void:
	if _message:
		_message.text = text
