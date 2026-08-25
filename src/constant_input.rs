use std::path::{Path, PathBuf};
use std::fs::read_to_string;
use std::env::{current_dir, current_exe};

use toml::Table;

/// Load constants from file or use defaults
pub struct Constants {
    pub fullscreen: bool,
    pub transparent_window: bool,
    pub clear_color: [f64; 4],
    pub update_rate: f64,
    pub max_simulation_time_per_update: f64,

    pub input_force: f32,
    pub decay_factor: f32,
    pub target_radius: f32,
    pub force_falloff: f32,

    pub point_count: u64,
    pub point_size: f32,
    pub point_corner_colors: [[f32; 4]; 4],
    pub points_circular: bool,
}
impl Default for Constants {
    fn default() -> Self {
        Self {
            fullscreen: true,
            transparent_window: false,
            clear_color: [0.0, 0.0, 0.0, 0.0],
            update_rate: 10000.0,
            // measured in frames
            max_simulation_time_per_update: 6.0,

            input_force: 0.0000003,
            decay_factor: 0.99987,
            target_radius: 0.2,
            force_falloff: 0.5,

            point_count: 1_000_000,
            point_size: 0.001,
            point_corner_colors: [
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
                [0.0, 1.0, 1.0, 1.0],
            ],
            points_circular: false,
        }
    }
}
impl Constants {
    pub fn new() -> Self {
        let mut rtn = Self::default();

        let search_paths = [
            current_dir().unwrap(),
            current_exe().unwrap().parent().unwrap().to_path_buf(),
        ];

        rtn = match rtn.try_from_toml_search_paths(&search_paths) {
            Ok(rtn) => rtn,
            Err(rtn) => rtn,
        };

        rtn
    }
    fn try_from_toml_search_paths(mut self, paths: &[PathBuf]) -> Result<Self, Self> {
        for path in paths {
            let mut path = path.to_path_buf();
            path.push("point_sim.toml");
            self = match self.try_from_toml(&path) {
                Ok(rtn) => return Ok(rtn),
                Err(rtn) => rtn,
            };
        }
        Err(self)
    }
    fn try_from_toml(mut self, file_path: &Path) -> Result<Self, Self> {
        match file_path.try_exists() {
            Ok(true) => {
                let table = match read_to_string(file_path).unwrap().parse::<Table>() {
                    Ok(table) => table,
                    Err(e) => { panic!("{}", e) },
                };
                if let Some(v) = table.get("fullscreen") {
                    self.fullscreen = v.as_bool().expect("`fullscreen` should be bool");
                }
                if let Some(v) = table.get("transparent_window") {
                    self.transparent_window = v.as_bool().expect("`transparent_window` should be bool");
                }
                if let Some(v) = table.get("clear_color") {
                    let clear_color = v.as_array().expect("`clear_color` should be array of 4 floats");
                    for i in 0..self.clear_color.len() {
                        self.clear_color[i] = clear_color[i].as_float()
                            .expect("`clear_color` should be array of 4 floats");
                    }
                }
                if let Some(v) = table.get("update_rate") {
                    self.update_rate = v.as_float().expect("`update_rate` should be float");
                }
                if let Some(v) = table.get("max_simulation_time_per_update") {
                    self.max_simulation_time_per_update = v.as_float()
                        .expect("`max_simulation_time_per_update` should be float");
                }
                if let Some(v) = table.get("input_force") {
                    self.input_force = v.as_float().expect("`input_force` should be float") as f32;
                }
                if let Some(v) = table.get("decay_factor") {
                    self.decay_factor = v.as_float().expect("`decay_factor` should be float") as f32;
                }
                if let Some(v) = table.get("target_radius") {
                    self.target_radius = v.as_float().expect("`target_radius` should be float") as f32;
                }
                if let Some(v) = table.get("force_falloff") {
                    self.force_falloff = v.as_float().expect("`force_falloff` should be float") as f32;
                }
                if let Some(v) = table.get("point_count") {
                    self.point_count = v.as_integer().expect("`point_count` should be float")
                        .try_into().unwrap();
                }
                if let Some(v) = table.get("point_size") {
                    self.point_size = v.as_float().expect("`point_size` should be float") as f32;
                }
                if let Some(v) = table.get("point_corner_colors") {
                    let point_corner_colors = v.as_array()
                        .expect("`point_corner_colors` should be 4x4 2D array of floats");
                    for i in 0..self.point_corner_colors.len() {
                        let point_corner_color = point_corner_colors[i].as_array()
                            .expect("`point_corner_colors` should be 4x4 2D array of floats");
                        for j in 0..self.point_corner_colors[0].len() {
                            self.point_corner_colors[i][j] = point_corner_color[j].as_float()
                                .expect("`point_corner_colors` should be array of 4 floats") as f32;
                        }
                    }
                }
                if let Some(v) = table.get("points_circular") {
                    self.points_circular = v.as_bool().expect("`points_circular` should be bool");
                }
                Ok(self)
            },
            _ => Err(self),
        }
    }
}
