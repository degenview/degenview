//! Dense `f64` matrix arithmetic for `matrix.pow`, `matrix.kron` and the linear algebra of
//! `matrix.*`. Pure and independent of the interpreter's values: `matrix_builtins` converts a
//! `matrix<>` to a `Dense` (an element that is not a number becomes NaN) and back (NaN becomes `na`).

/// A dense row-major matrix of `f64`.
#[derive(Clone, Debug, PartialEq)]
pub struct Dense {
    rows: usize,
    columns: usize,
    data: Vec<f64>,
}

impl Dense {
    /// A matrix from its elements row by row; `data.len()` must be `rows * columns`.
    pub fn new(rows: usize, columns: usize, data: Vec<f64>) -> Self {
        assert_eq!(data.len(), rows * columns, "a matrix is rows × columns");
        Self {
            rows,
            columns,
            data,
        }
    }

    pub fn identity(size: usize) -> Self {
        let mut identity = Self::new(size, size, vec![0.0; size * size]);
        for index in 0..size {
            identity.data[index * size + index] = 1.0;
        }
        identity
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn data(&self) -> &[f64] {
        &self.data
    }

    pub fn get(&self, row: usize, column: usize) -> f64 {
        self.data[row * self.columns + column]
    }

    /// `self × other`; the inner dimensions must agree.
    pub fn multiply(&self, other: &Dense) -> Dense {
        assert_eq!(self.columns, other.rows, "inner dimensions agree");
        let mut data = vec![0.0; self.rows * other.columns];
        for row in 0..self.rows {
            for inner in 0..self.columns {
                let factor = self.get(row, inner);
                for column in 0..other.columns {
                    data[row * other.columns + column] += factor * other.get(inner, column);
                }
            }
        }
        Dense::new(self.rows, other.columns, data)
    }

    /// A square matrix times itself `power` times (the identity for 0), by repeated squaring.
    pub fn power(&self, mut power: u64) -> Dense {
        assert_eq!(self.rows, self.columns, "only a square matrix has powers");
        let mut result = Dense::identity(self.rows);
        let mut base = self.clone();
        while power > 0 {
            if power & 1 == 1 {
                result = result.multiply(&base);
            }
            power >>= 1;
            if power > 0 {
                base = base.multiply(&base);
            }
        }
        result
    }

    /// The Kronecker product: every element of `self` scales a copy of `other`.
    pub fn kronecker(&self, other: &Dense) -> Dense {
        let (rows, columns) = (self.rows * other.rows, self.columns * other.columns);
        let mut data = vec![0.0; rows * columns];
        for row in 0..self.rows {
            for column in 0..self.columns {
                let factor = self.get(row, column);
                for inner_row in 0..other.rows {
                    for inner_column in 0..other.columns {
                        data[(row * other.rows + inner_row) * columns
                            + column * other.columns
                            + inner_column] = factor * other.get(inner_row, inner_column);
                    }
                }
            }
        }
        Dense::new(rows, columns, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense(rows: usize, columns: usize, data: &[f64]) -> Dense {
        Dense::new(rows, columns, data.to_vec())
    }

    #[test]
    fn multiplication_follows_the_row_by_column_rule() {
        let a = dense(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let b = dense(3, 2, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        assert_eq!(a.multiply(&b), dense(2, 2, &[58.0, 64.0, 139.0, 154.0]));
    }

    #[test]
    fn a_power_is_repeated_multiplication() {
        let m = dense(2, 2, &[1.0, 1.0, 1.0, 0.0]);
        assert_eq!(m.power(0), Dense::identity(2));
        assert_eq!(m.power(1), m);
        // The Fibonacci matrix: [[F(n+1), F(n)], [F(n), F(n-1)]].
        assert_eq!(m.power(10), dense(2, 2, &[89.0, 55.0, 55.0, 34.0]));
    }

    #[test]
    fn the_kronecker_product_tiles_scaled_copies() {
        let a = dense(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        let b = dense(1, 2, &[0.0, 5.0]);
        assert_eq!(
            a.kronecker(&b),
            dense(2, 4, &[0.0, 5.0, 0.0, 10.0, 0.0, 15.0, 0.0, 20.0])
        );
        let c = dense(2, 1, &[1.0, 2.0]);
        assert_eq!(b.kronecker(&c), dense(2, 2, &[0.0, 5.0, 0.0, 10.0]));
    }
}
