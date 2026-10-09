class_name OrbitCamera
extends Camera3D
## Orbits around a target point.
## Mouse: left-drag rotates, right/middle-drag pans, wheel zooms.
## Touch: one finger rotates, two fingers pinch to zoom and drag to pan.

var target := Vector3.ZERO
var distance := 30.0
var yaw := -0.6
var pitch := 0.8

var _touches: Dictionary = {}  # finger index -> position


func frame(center: Vector3, size: float) -> void:
	target = center
	distance = maxf(size * 1.1, 6.0)
	_update()


func _ready() -> void:
	_update()


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseMotion:
		var motion := event as InputEventMouseMotion
		if _touches.size() >= 2:
			return  # the first finger of a pinch also arrives as mouse motion
		if motion.button_mask & MOUSE_BUTTON_MASK_LEFT:
			_rotate(motion.relative)
		elif motion.button_mask & (MOUSE_BUTTON_MASK_RIGHT | MOUSE_BUTTON_MASK_MIDDLE):
			_pan(motion.relative)
	elif event is InputEventMouseButton and event.pressed:
		match event.button_index:
			MOUSE_BUTTON_WHEEL_UP:
				_zoom(0.9)
			MOUSE_BUTTON_WHEEL_DOWN:
				_zoom(1.1)
	elif event is InputEventMagnifyGesture:
		_zoom(1.0 / event.factor)
	elif event is InputEventPanGesture:
		_zoom(1.0 + event.delta.y * 0.05)
	elif event is InputEventScreenTouch:
		if event.pressed:
			_touches[event.index] = event.position
		else:
			_touches.erase(event.index)
	elif event is InputEventScreenDrag:
		_touch_drag(event)


func _touch_drag(event: InputEventScreenDrag) -> void:
	if _touches.size() == 2 and _touches.has(event.index):
		# Pinch: zoom by the change in finger distance, pan by the midpoint's motion.
		var other: Vector2 = Vector2.ZERO
		for i: int in _touches:
			if i != event.index:
				other = _touches[i]
		var before: float = (_touches[event.index] as Vector2).distance_to(other)
		var after: float = event.position.distance_to(other)
		if before > 1.0 and after > 1.0:
			_zoom(before / after)
		_pan(event.relative / 2.0)
		_touches[event.index] = event.position
	# One finger arrives as emulated mouse motion (left button) and rotates.
	elif _touches.has(event.index):
		_touches[event.index] = event.position


func _rotate(relative: Vector2) -> void:
	yaw -= relative.x * 0.006
	pitch = clampf(pitch + relative.y * 0.006, 0.08, 1.5)
	_update()


func _pan(relative: Vector2) -> void:
	var scale := distance * 0.0015
	var right := global_transform.basis.x
	var forward := Vector3(-sin(yaw), 0.0, -cos(yaw))
	target += (-right * relative.x + forward * relative.y) * scale
	_update()


func _zoom(factor: float) -> void:
	distance = clampf(distance * factor, 2.0, 2000.0)
	_update()


func _update() -> void:
	var offset := Vector3(
		sin(yaw) * cos(pitch),
		sin(pitch),
		cos(yaw) * cos(pitch),
	) * distance
	look_at_from_position(target + offset, target, Vector3.UP)
