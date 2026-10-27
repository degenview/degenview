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

    pub fn transpose(&self) -> Dense {
        let mut data = vec![0.0; self.data.len()];
        for row in 0..self.rows {
            for column in 0..self.columns {
                data[column * self.rows + row] = self.get(row, column);
            }
        }
        Dense::new(self.columns, self.rows, data)
    }

    fn largest_magnitude(&self) -> f64 {
        self.data
            .iter()
            .fold(0.0, |largest, value| largest.max(value.abs()))
    }

    /// The matrix equals its transpose exactly.
    pub fn is_symmetric(&self) -> bool {
        self.rows == self.columns
            && (0..self.rows)
                .all(|row| (0..row).all(|column| self.get(row, column) == self.get(column, row)))
    }

    /// LU decomposition with partial pivoting of a square matrix: the packed factors (unit lower
    /// below the diagonal, upper on and above it), the row permutation, and the sign of that
    /// permutation. `None` when a pivot is exactly zero.
    fn lu(&self) -> Option<(Vec<f64>, Vec<usize>, f64)> {
        assert_eq!(self.rows, self.columns, "only a square matrix has an LU");
        let n = self.rows;
        let mut lu = self.data.clone();
        let mut permutation: Vec<usize> = (0..n).collect();
        let mut sign = 1.0;
        for column in 0..n {
            let pivot = (column..n).max_by(|a, b| {
                lu[a * n + column]
                    .abs()
                    .total_cmp(&lu[b * n + column].abs())
            })?;
            if lu[pivot * n + column] == 0.0 {
                return None;
            }
            if pivot != column {
                for k in 0..n {
                    lu.swap(pivot * n + k, column * n + k);
                }
                permutation.swap(pivot, column);
                sign = -sign;
            }
            for row in column + 1..n {
                let factor = lu[row * n + column] / lu[column * n + column];
                lu[row * n + column] = factor;
                for k in column + 1..n {
                    lu[row * n + k] -= factor * lu[column * n + k];
                }
            }
        }
        Some((lu, permutation, sign))
    }

    /// The determinant of a square matrix, from its LU decomposition (1 for the empty matrix).
    pub fn determinant(&self) -> f64 {
        match self.lu() {
            None => 0.0,
            Some((lu, _, sign)) => {
                let n = self.rows;
                (0..n).fold(sign, |product, index| product * lu[index * n + index])
            }
        }
    }

    /// The inverse of a square matrix by LU decomposition; `None` when it is singular, meaning a
    /// pivot is zero or smaller than `n · ε` times the largest element.
    pub fn inverse(&self) -> Option<Dense> {
        let n = self.rows;
        let (lu, permutation, _) = self.lu()?;
        let tolerance = n as f64 * f64::EPSILON * self.largest_magnitude();
        if (0..n).any(|index| lu[index * n + index].abs() <= tolerance) {
            return None;
        }
        let mut inverse = vec![0.0; n * n];
        let mut column = vec![0.0; n];
        for target in 0..n {
            // Solve L·U·x = P·e_target.
            for row in 0..n {
                let mut value = if permutation[row] == target { 1.0 } else { 0.0 };
                for k in 0..row {
                    value -= lu[row * n + k] * column[k];
                }
                column[row] = value;
            }
            for row in (0..n).rev() {
                let mut value = column[row];
                for k in row + 1..n {
                    value -= lu[row * n + k] * column[k];
                }
                column[row] = value / lu[row * n + row];
            }
            for row in 0..n {
                inverse[row * n + target] = column[row];
            }
        }
        Some(Dense::new(n, n, inverse))
    }

    /// The singular value decomposition `A = U·diag(σ)·Vᵀ` by one-sided Jacobi rotations, for a
    /// matrix with at least as many rows as columns: `U` is rows × columns with orthonormal
    /// columns wherever σ > 0, `V` is columns × columns, and σ is sorted descending.
    fn svd_tall(&self) -> (Dense, Vec<f64>, Dense) {
        assert!(self.rows >= self.columns, "svd_tall takes a tall matrix");
        let (m, n) = (self.rows, self.columns);
        let mut u = self.data.clone();
        let mut v = Dense::identity(n).data;
        for _ in 0..60 {
            let mut rotated = false;
            for p in 0..n {
                for q in p + 1..n {
                    let (mut alpha, mut beta, mut gamma) = (0.0, 0.0, 0.0);
                    for i in 0..m {
                        alpha += u[i * n + p] * u[i * n + p];
                        beta += u[i * n + q] * u[i * n + q];
                        gamma += u[i * n + p] * u[i * n + q];
                    }
                    if gamma == 0.0 || gamma.abs() <= f64::EPSILON * (alpha * beta).sqrt() {
                        continue;
                    }
                    rotated = true;
                    let zeta = (beta - alpha) / (2.0 * gamma);
                    let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                    let c = 1.0 / (1.0 + t * t).sqrt();
                    let s = c * t;
                    for i in 0..m {
                        let (x, y) = (u[i * n + p], u[i * n + q]);
                        u[i * n + p] = c * x - s * y;
                        u[i * n + q] = s * x + c * y;
                    }
                    for i in 0..n {
                        let (x, y) = (v[i * n + p], v[i * n + q]);
                        v[i * n + p] = c * x - s * y;
                        v[i * n + q] = s * x + c * y;
                    }
                }
            }
            if !rotated {
                break;
            }
        }
        let mut values: Vec<f64> = (0..n)
            .map(|j| {
                (0..m)
                    .map(|i| u[i * n + j] * u[i * n + j])
                    .sum::<f64>()
                    .sqrt()
            })
            .collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
        let mut sorted_u = vec![0.0; m * n];
        let mut sorted_v = vec![0.0; n * n];
        for (target, source) in order.iter().enumerate() {
            let sigma = values[*source];
            for i in 0..m {
                sorted_u[i * n + target] = if sigma > 0.0 {
                    u[i * n + source] / sigma
                } else {
                    0.0
                };
            }
            for i in 0..n {
                sorted_v[i * n + target] = v[i * n + source];
            }
        }
        values = order.iter().map(|source| values[*source]).collect();
        (
            Dense::new(m, n, sorted_u),
            values,
            Dense::new(n, n, sorted_v),
        )
    }

    /// The singular values, largest first.
    pub fn singular_values(&self) -> Vec<f64> {
        if self.rows >= self.columns {
            self.svd_tall().1
        } else {
            self.transpose().svd_tall().1
        }
    }

    /// The number of singular values above `max(rows, columns) · σmax · ε`.
    pub fn rank(&self) -> usize {
        let values = self.singular_values();
        let largest = values.first().copied().unwrap_or(0.0);
        let tolerance = self.rows.max(self.columns) as f64 * largest * f64::EPSILON;
        values.iter().filter(|value| **value > tolerance).count()
    }

    /// The Moore–Penrose pseudoinverse, `V·Σ⁺·Uᵀ`; the inverse itself for a non-singular square
    /// matrix.
    pub fn pseudo_inverse(&self) -> Dense {
        if self.rows == self.columns
            && let Some(inverse) = self.inverse()
        {
            return inverse;
        }
        if self.rows < self.columns {
            return self.transpose().pseudo_inverse().transpose();
        }
        let (u, values, v) = self.svd_tall();
        let largest = values.first().copied().unwrap_or(0.0);
        let tolerance = self.rows.max(self.columns) as f64 * largest * f64::EPSILON;
        let (m, n) = (self.rows, self.columns);
        let mut data = vec![0.0; n * m];
        for (k, sigma) in values.iter().enumerate() {
            if *sigma <= tolerance {
                continue;
            }
            for row in 0..n {
                for column in 0..m {
                    data[row * m + column] += v.get(row, k) * u.get(column, k) / sigma;
                }
            }
        }
        Dense::new(n, m, data)
    }

    /// The eigenvalues (ascending) and eigenvectors (the columns, unit length) of a symmetric
    /// matrix, by Householder reduction to tridiagonal form followed by the implicit QL
    /// algorithm (the EISPACK `tred2`/`tql2` pair TradingView names). `None` if the QL iteration
    /// does not converge.
    pub fn symmetric_eigen(&self) -> Option<(Vec<f64>, Dense)> {
        assert!(
            self.is_symmetric(),
            "the QL algorithm needs a symmetric matrix"
        );
        let n = self.rows;
        if n == 0 {
            return Some((Vec::new(), self.clone()));
        }
        let mut v: Vec<Vec<f64>> = (0..n)
            .map(|row| (0..n).map(|column| self.get(row, column)).collect())
            .collect();
        let mut d = vec![0.0; n];
        let mut e = vec![0.0; n];
        tridiagonalize(&mut v, &mut d, &mut e);
        quadratic_l(&mut v, &mut d, &mut e)?;
        let data = (0..n).flat_map(|row| v[row].clone()).collect();
        Some((d, Dense::new(n, n, data)))
    }
}

/// Householder reduction of the symmetric matrix in `v` to tridiagonal form (EISPACK `tred2`):
/// afterwards `d` is the diagonal, `e` the subdiagonal (from index 1) and `v` the accumulated
/// orthogonal transformation. The loops index several arrays by the same running indices, as the
/// published algorithm does, which is how it stays checkable against it.
#[allow(clippy::needless_range_loop)]
fn tridiagonalize(v: &mut [Vec<f64>], d: &mut [f64], e: &mut [f64]) {
    let n = d.len();
    d.copy_from_slice(&v[n - 1]);
    for i in (1..n).rev() {
        let scale: f64 = d[..i].iter().map(|value| value.abs()).sum();
        let mut h = 0.0;
        if scale == 0.0 {
            e[i] = d[i - 1];
            for j in 0..i {
                d[j] = v[i - 1][j];
                v[i][j] = 0.0;
                v[j][i] = 0.0;
            }
        } else {
            for k in 0..i {
                d[k] /= scale;
                h += d[k] * d[k];
            }
            let mut f = d[i - 1];
            let mut g = h.sqrt();
            if f > 0.0 {
                g = -g;
            }
            e[i] = scale * g;
            h -= f * g;
            d[i - 1] = f - g;
            for value in e.iter_mut().take(i) {
                *value = 0.0;
            }
            for j in 0..i {
                f = d[j];
                v[j][i] = f;
                g = e[j] + v[j][j] * f;
                for k in j + 1..i {
                    g += v[k][j] * d[k];
                    e[k] += v[k][j] * f;
                }
                e[j] = g;
            }
            f = 0.0;
            for j in 0..i {
                e[j] /= h;
                f += e[j] * d[j];
            }
            let hh = f / (h + h);
            for j in 0..i {
                e[j] -= hh * d[j];
            }
            for j in 0..i {
                f = d[j];
                g = e[j];
                for k in j..i {
                    v[k][j] -= f * e[k] + g * d[k];
                }
                d[j] = v[i - 1][j];
                v[i][j] = 0.0;
            }
        }
        d[i] = h;
    }
    for i in 0..n - 1 {
        v[n - 1][i] = v[i][i];
        v[i][i] = 1.0;
        let h = d[i + 1];
        if h != 0.0 {
            for k in 0..=i {
                d[k] = v[k][i + 1] / h;
            }
            for j in 0..=i {
                let mut g = 0.0;
                for k in 0..=i {
                    g += v[k][i + 1] * v[k][j];
                }
                for k in 0..=i {
                    v[k][j] -= g * d[k];
                }
            }
        }
        for k in 0..=i {
            v[k][i + 1] = 0.0;
        }
    }
    for j in 0..n {
        d[j] = v[n - 1][j];
        v[n - 1][j] = 0.0;
    }
    v[n - 1][n - 1] = 1.0;
    e[0] = 0.0;
}

/// The implicit QL iteration on the tridiagonal `d`/`e` (EISPACK `tql2`), accumulating into `v`
/// and leaving the eigenvalues sorted ascending with their vectors. `None` after too many
/// iterations on one eigenvalue.
fn quadratic_l(v: &mut [Vec<f64>], d: &mut [f64], e: &mut [f64]) -> Option<()> {
    const MAX_ITERATIONS: usize = 200;
    let n = d.len();
    for i in 1..n {
        e[i - 1] = e[i];
    }
    e[n - 1] = 0.0;
    let mut f = 0.0;
    let mut tst1: f64 = 0.0;
    let eps = f64::EPSILON;
    for l in 0..n {
        tst1 = tst1.max(d[l].abs() + e[l].abs());
        let mut m = l;
        while m < n {
            if e[m].abs() <= eps * tst1 {
                break;
            }
            m += 1;
        }
        if m > l {
            let mut iterations = 0;
            loop {
                iterations += 1;
                if iterations > MAX_ITERATIONS {
                    return None;
                }
                let mut g = d[l];
                let mut p = (d[l + 1] - g) / (2.0 * e[l]);
                let mut r = p.hypot(1.0);
                if p < 0.0 {
                    r = -r;
                }
                d[l] = e[l] / (p + r);
                d[l + 1] = e[l] * (p + r);
                let dl1 = d[l + 1];
                let mut h = g - d[l];
                for value in d.iter_mut().skip(l + 2) {
                    *value -= h;
                }
                f += h;
                p = d[m];
                let mut c = 1.0;
                let mut c2 = c;
                let mut c3 = c;
                let el1 = e[l + 1];
                let mut s = 0.0;
                let mut s2 = 0.0;
                for i in (l..m).rev() {
                    c3 = c2;
                    c2 = c;
                    s2 = s;
                    g = c * e[i];
                    h = c * p;
                    r = p.hypot(e[i]);
                    e[i + 1] = s * r;
                    s = e[i] / r;
                    c = p / r;
                    p = c * d[i] - s * g;
                    d[i + 1] = h + s * (c * g + s * d[i]);
                    for row in v.iter_mut() {
                        h = row[i + 1];
                        row[i + 1] = s * row[i] + c * h;
                        row[i] = c * row[i] - s * h;
                    }
                }
                p = -s * s2 * c3 * el1 * e[l] / dl1;
                e[l] = s * p;
                d[l] = c * p;
                if e[l].abs() <= eps * tst1 {
                    break;
                }
            }
        }
        d[l] += f;
        e[l] = 0.0;
    }
    for i in 0..n - 1 {
        let (mut k, mut p) = (i, d[i]);
        for (j, value) in d.iter().enumerate().skip(i + 1) {
            if *value < p {
                k = j;
                p = *value;
            }
        }
        if k != i {
            d[k] = d[i];
            d[i] = p;
            for row in v.iter_mut() {
                row.swap(i, k);
            }
        }
    }
    Some(())
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

    fn close(a: &Dense, b: &Dense, tolerance: f64) -> bool {
        a.rows() == b.rows()
            && a.columns() == b.columns()
            && a.data()
                .iter()
                .zip(b.data())
                .all(|(x, y)| (x - y).abs() <= tolerance)
    }

    /// A deterministic, well-conditioned test matrix with no structure.
    fn scrambled(rows: usize, columns: usize) -> Dense {
        let data = (0..rows * columns)
            .map(|index| ((index * 7 + 3) % 11) as f64 - 4.0 + (index % 3) as f64 * 0.25)
            .collect();
        Dense::new(rows, columns, data)
    }

    #[test]
    fn determinants_follow_the_definition() {
        assert_eq!(dense(2, 2, &[2.0, 0.0, 0.0, 3.0]).determinant(), 6.0);
        assert!((dense(2, 2, &[1.0, 2.0, 3.0, 4.0]).determinant() + 2.0).abs() < 1e-12);
        let m = dense(3, 3, &[6.0, 1.0, 1.0, 4.0, -2.0, 5.0, 2.0, 8.0, 7.0]);
        assert!((m.determinant() + 306.0).abs() < 1e-9);
        assert_eq!(dense(2, 2, &[1.0, 2.0, 2.0, 4.0]).determinant(), 0.0);
        assert_eq!(dense(0, 0, &[]).determinant(), 1.0);
        // A row swap flips the sign.
        let swapped = dense(2, 2, &[3.0, 4.0, 1.0, 2.0]);
        assert!((swapped.determinant() - 2.0).abs() < 1e-12);
    }

    #[test]
    fn an_inverse_undoes_the_matrix_and_a_singular_one_has_none() {
        let m = dense(2, 2, &[4.0, 7.0, 2.0, 6.0]);
        assert!(close(
            &m.inverse().unwrap(),
            &dense(2, 2, &[0.6, -0.7, -0.2, 0.4]),
            1e-12
        ));
        let big = scrambled(5, 5);
        let inverse = big.inverse().expect("the test matrix is invertible");
        assert!(close(&big.multiply(&inverse), &Dense::identity(5), 1e-9));
        assert!(dense(2, 2, &[1.0, 2.0, 2.0, 4.0]).inverse().is_none());
        assert!(dense(3, 3, &[0.0; 9]).inverse().is_none());
        // Nearly singular rows are caught by the tolerance, not only an exact zero pivot.
        let near = dense(2, 2, &[1.0, 2.0, 1.0 + 1e-17, 2.0]);
        assert!(near.inverse().is_none());
    }

    #[test]
    fn rank_counts_the_independent_rows() {
        assert_eq!(Dense::identity(4).rank(), 4);
        assert_eq!(dense(2, 2, &[1.0, 2.0, 2.0, 4.0]).rank(), 1);
        assert_eq!(
            dense(3, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]).rank(),
            2
        );
        assert_eq!(dense(2, 3, &[1.0, 0.0, 1.0, 0.0, 1.0, 1.0]).rank(), 2);
        assert_eq!(dense(3, 2, &[1.0, 2.0, 2.0, 4.0, 3.0, 6.0]).rank(), 1);
        assert_eq!(dense(2, 2, &[0.0; 4]).rank(), 0);
        assert_eq!(dense(0, 0, &[]).rank(), 0);
    }

    #[test]
    fn singular_values_are_sorted_and_reproduce_the_matrix_norm() {
        let m = dense(2, 2, &[3.0, 0.0, 0.0, -2.0]);
        let values = m.singular_values();
        assert!((values[0] - 3.0).abs() < 1e-12 && (values[1] - 2.0).abs() < 1e-12);
        let tall = scrambled(6, 3);
        let (u, sigma, v) = tall.svd_tall();
        assert!(sigma.windows(2).all(|pair| pair[0] >= pair[1]));
        let mut diagonal = Dense::new(3, 3, vec![0.0; 9]);
        for (index, value) in sigma.iter().enumerate() {
            diagonal.data[index * 3 + index] = *value;
        }
        let rebuilt = u.multiply(&diagonal).multiply(&v.transpose());
        assert!(close(&rebuilt, &tall, 1e-9));
    }

    #[test]
    fn the_pseudoinverse_satisfies_the_moore_penrose_conditions() {
        for (rows, columns) in [(3, 2), (2, 3), (4, 4), (1, 3)] {
            let a = scrambled(rows, columns);
            let plus = a.pseudo_inverse();
            assert_eq!((plus.rows(), plus.columns()), (columns, rows));
            assert!(
                close(&a.multiply(&plus).multiply(&a), &a, 1e-8),
                "A·A⁺·A = A"
            );
            assert!(
                close(&plus.multiply(&a).multiply(&plus), &plus, 1e-8),
                "A⁺·A·A⁺ = A⁺"
            );
        }
        // Rank-deficient: the zero direction stays zero.
        let flat = dense(2, 2, &[1.0, 0.0, 0.0, 0.0]);
        assert!(close(&flat.pseudo_inverse(), &flat, 1e-12));
        // A non-singular square matrix gets its inverse.
        let m = dense(2, 2, &[4.0, 7.0, 2.0, 6.0]);
        assert_eq!(m.pseudo_inverse(), m.inverse().unwrap());
    }

    #[test]
    fn symmetric_eigen_finds_values_ascending_with_unit_vectors() {
        let (values, vectors) = dense(2, 2, &[2.0, 1.0, 1.0, 2.0])
            .symmetric_eigen()
            .unwrap();
        assert!((values[0] - 1.0).abs() < 1e-12 && (values[1] - 3.0).abs() < 1e-12);
        // Each column is an eigenvector of unit length.
        let a = dense(2, 2, &[2.0, 1.0, 1.0, 2.0]);
        for (k, lambda) in values.iter().enumerate() {
            let column = Dense::new(2, 1, vec![vectors.get(0, k), vectors.get(1, k)]);
            let image = a.multiply(&column);
            let scaled = Dense::new(2, 1, column.data().iter().map(|x| x * lambda).collect());
            assert!(close(&image, &scaled, 1e-12));
            assert!((column.data().iter().map(|x| x * x).sum::<f64>() - 1.0).abs() < 1e-12);
        }
        let (diagonal, _) = dense(3, 3, &[5.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 2.0])
            .symmetric_eigen()
            .unwrap();
        assert_eq!(diagonal, [-1.0, 2.0, 5.0]);
        let (single, vector) = dense(1, 1, &[7.0]).symmetric_eigen().unwrap();
        assert_eq!((single, vector.data()), (vec![7.0], &[1.0][..]));
        let (repeated, _) = Dense::identity(3).symmetric_eigen().unwrap();
        assert_eq!(repeated, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_symmetric_matrix_is_rebuilt_from_its_eigen_decomposition() {
        // Hilbert matrices are symmetric and badly conditioned: a stern test of the QL iteration.
        for n in [3usize, 6] {
            let hilbert = Dense::new(
                n,
                n,
                (0..n * n)
                    .map(|index| 1.0 / ((index / n + index % n + 1) as f64))
                    .collect(),
            );
            let (values, vectors) = hilbert.symmetric_eigen().unwrap();
            assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
            let mut diagonal = Dense::new(n, n, vec![0.0; n * n]);
            for (index, value) in values.iter().enumerate() {
                diagonal.data[index * n + index] = *value;
            }
            let rebuilt = vectors.multiply(&diagonal).multiply(&vectors.transpose());
            assert!(close(&rebuilt, &hilbert, 1e-12), "n = {n}");
            assert!(close(
                &vectors.transpose().multiply(&vectors),
                &Dense::identity(n),
                1e-12
            ));
        }
        let covariance = dense(3, 3, &[4.0, 1.0, 0.0, 1.0, 3.0, 1.0, 0.0, 1.0, 2.0]);
        let (values, _) = covariance.symmetric_eigen().unwrap();
        // The trace and determinant are the sum and product of the eigenvalues.
        assert!((values.iter().sum::<f64>() - 9.0).abs() < 1e-12);
        assert!((values.iter().product::<f64>() - covariance.determinant()).abs() < 1e-10);
    }

    #[test]
    fn symmetry_is_exact_equality_with_the_transpose() {
        assert!(dense(2, 2, &[1.0, 2.0, 2.0, 3.0]).is_symmetric());
        assert!(!dense(2, 2, &[1.0, 2.0, 2.5, 3.0]).is_symmetric());
        assert!(!dense(1, 2, &[1.0, 1.0]).is_symmetric());
    }
}
