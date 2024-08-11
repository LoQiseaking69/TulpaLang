pub struct Kinematics;

impl Kinematics {
    /// Calculate velocity given initial velocity, acceleration, and time.
    pub fn calculate_velocity(initial_velocity: f64, acceleration: f64, time: f64) -> f64 {
        initial_velocity + acceleration * time
    }

    /// Calculate position given initial position, initial velocity, acceleration, and time.
    pub fn calculate_position(
        initial_position: f64,
        initial_velocity: f64,
        acceleration: f64,
        time: f64,
    ) -> f64 {
        initial_position + initial_velocity * time + 0.5 * acceleration * time.powi(2)
    }

    /// Calculate acceleration given change in velocity and time.
    pub fn calculate_acceleration(change_in_velocity: f64, time: f64) -> f64 {
        change_in_velocity / time
    }

    /// Calculate the time of flight for projectile motion given initial velocity and angle.
    pub fn calculate_time_of_flight(initial_velocity: f64, angle: f64) -> f64 {
        let g = 9.81; // Acceleration due to gravity (m/s^2)
        (2.0 * initial_velocity * angle.to_radians().sin()) / g
    }

    /// Calculate the range of a projectile given initial velocity and angle.
    pub fn calculate_range(initial_velocity: f64, angle: f64) -> f64 {
        let g = 9.81; // Acceleration due to gravity (m/s^2)
        (initial_velocity.powi(2) * (2.0 * angle).to_radians().sin()) / g
    }

    /// Calculate the maximum height of a projectile given initial velocity and angle.
    pub fn calculate_max_height(initial_velocity: f64, angle: f64) -> f64 {
        let g = 9.81; // Acceleration due to gravity (m/s^2)
        (initial_velocity.powi(2) * angle.to_radians().sin().powi(2)) / (2.0 * g)
    }
}
