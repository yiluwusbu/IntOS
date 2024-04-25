use crate::benchmarks::riotbench::ValueType;

pub struct SimpleRegression {
    sum_x: ValueType,
    sum_xx: ValueType,
    sum_y: ValueType,
    sum_yy: ValueType,
    sum_xy: ValueType,
    n: usize,
    x_bar: ValueType,
    y_bar: ValueType,
    has_intercept: bool,
}

impl SimpleRegression {
    pub fn new(has_intercept: bool) -> Self {
        SimpleRegression {
            sum_x: 0,
            sum_xx: 0,
            sum_y: 0,
            sum_yy: 0,
            sum_xy: 0,
            n: 0,
            x_bar: 0,
            y_bar: 0,
            has_intercept,
        }
    }
    pub fn add_data(&mut self, x: ValueType, y: ValueType) {
        if self.n == 0 {
            self.x_bar = x;
            self.y_bar = y;
        } else if self.has_intercept {
            let fact1 = 1 + self.n as ValueType;
            let denom = fact1;
            let dx = x - self.x_bar;
            let dy = y - self.y_bar;
            self.sum_xx += dx * dx * self.n as ValueType / denom;
            self.sum_yy += dy * dy * self.n as ValueType / denom;
            self.sum_xy += dx * dy * self.n as ValueType / denom;
            self.x_bar += dx / fact1;
            self.y_bar += dy / fact1;
        }

        if !self.has_intercept {
            self.sum_xx += x * x;
            self.sum_yy += y * y;
            self.sum_xy += x * y;
        }

        self.sum_x += x;
        self.sum_y += y;
        self.n += 1;
    }

    pub fn remove_data(&mut self, x: ValueType, y: ValueType) {
        if self.n > 0 {
            let fact1 = if self.n == 1 {
                ValueType::MAX
            } else {
                self.n as ValueType - 1
            };

            if self.has_intercept {
                let dx = x - self.x_bar;
                let dy = y - self.y_bar;
                self.sum_xx -= dx * dx * self.n as ValueType / fact1;
                self.sum_yy -= dy * dy * self.n as ValueType / fact1;
                self.sum_xy -= dx * dy * self.n as ValueType / fact1;
                self.x_bar -= dx / fact1;
                self.y_bar -= dy / fact1;
            } else {
                self.sum_xx -= x * x;
                self.sum_yy -= y * y;
                self.sum_xy -= x * y;
                self.x_bar -= x / fact1;
                self.y_bar -= y / fact1;
            }

            self.sum_x -= x;
            self.sum_y -= y;
            self.n -= 1;
        }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    pub fn get_slope(&self) -> ValueType {
        if self.n < 2 {
            ValueType::MAX
        } else {
            if self.sum_xx == 0 {
                ValueType::MAX
            } else {
                self.sum_xy / self.sum_xx
            }
        }
    }

    pub fn get_intercept(&self) -> ValueType {
        if self.has_intercept {
            let slope = self.get_slope();
            (self.sum_y - slope * self.sum_x) / self.n as ValueType
        } else {
            0
        }
    }

    pub fn predict(&self, x: ValueType) -> ValueType {
        let slope = self.get_slope();
        slope * x + self.get_intercept()
    }
}
