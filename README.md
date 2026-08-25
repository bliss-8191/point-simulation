# Point Simulation

Graphics art project directly inspired by a project I saw at 2026 [Open Sauce](https://www.opensauce.com/).

Simulates many points that each have position and velocity.

![GIF of simulation with large points.](https://i.imgur.com/40Z1WhE.gif)
![GIF of simulation with medium points.](https://i.imgur.com/9GaFLWS.gif)
![GIF of simulation with small points.](https://i.imgur.com/NFLNhp6.gif)
![GIF of simulation with steep force falloff.](https://i.imgur.com/G1xYJnJ.gif)

Transparent window support:

![GIF of simulation with recording software in the background.](https://i.imgur.com/YvjfdqB.gif)

## Settings

The application searches the present working directory, then searches the executable directory for a file called [`point_sim.toml`](./point_sim.toml).
Window settings, visual settings, and simulation settings can all be changed from here.

The `R` key resets the simulation and reloads the configuration file. The only exception is that window settings require restart (fullscreen & transparency).

Each point is drawn as a square. `point_corner_colors` determines the colors of each corner. The format is an array of 4 RGBA colors.
Note: the alpha component is only used with the transparent window setting. Points don't alpha blend with each other or with the background.

### Performance:

Rather than having a variable delta time parameter per update, the simulation has a fixed delta time with a variable number of updates. The `update_rate` parameter sets the number of updates per second.
`max_simulation_time_per_update` is used to bound the amount the simulation can advance in one frame. This is to prevent a runaway situation that occurs if `update_rate` is higher than what the computer can handle.
`max_simulation_time_per_update` is measured in "intended" frames. That is, if the target framerate is 60 fps, a `max_simulation_time_per_update` of 6 means 0.1 seconds worth of updates may run in one actual frame.

In testing, the most important parameters for performance are:

* `point_count`
  * pretty self-explanatory, more points means more work to draw them
* `point_size`
  * In testing, it seemed the bottleneck was the number of pixels generated before the fragment stage.

Potentially, `update_rate` could also be very important in some situations. Increasing from the default only noticeably effected performance when I tested 100,000,000 points.

### Defaults:

```toml
fullscreen = true
# needs to be set with clear_color to work
transparent_window = false
# color of background
clear_color = [0.0, 0.0, 0.0, 0.0]
# number of simulation updates per second
update_rate = 10000.0
# measured in frames
# setting this to N means the simulation can update N frames worth of updates in one frame
# this is used to keep simulation speed constant when the framerate is slow
max_simulation_time_per_update = 20.0

# how strong the input pushes and pulls points
input_force = 0.0000003
# how quickly point velocities decay
decay_factor = 0.99987
# the distance from the input points move toward
target_radius = 0.25
# decreases force factor if points are far. set to 0.0 to disable
force_falloff = 0.5

point_count = 1_000_000
point_size = 0.001
# colors of corners of points
point_corner_colors = [
    [1.0, 0.0, 0.0, 1.0],
    [1.0, 1.0, 0.0, 1.0],
    [0.0, 0.0, 1.0, 1.0],
    [0.0, 1.0, 1.0, 1.0],
]
# makes the points circular instead of square
points_circular = false
```
