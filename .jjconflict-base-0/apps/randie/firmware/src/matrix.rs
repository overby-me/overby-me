//! Small fixed-point matrices, for the data-fusion filter.
//!
//! The C carried separate matrix and vector entry points that computed the same
//! thing by different loops (`add_mat_mat` and `add_vec_vec`, `mult_mat_mat`
//! and `mult_mat_vec`). A vector here is a matrix with one row or one column,
//! so one set of operations covers both.
//!
//! Where the C returned `NULL` on a size mismatch and logged, these return
//! `None`. Where it returned `-1` from an out-of-bounds `matrix_get` -- a raw
//! word, which reads back as a perfectly plausible -0.000015 -- indexing here
//! panics, as indexing does everywhere else in Rust.

use alloc::vec;
use alloc::vec::Vec;

use crate::fix16::Fix16;

/// A rows-by-columns matrix of fixed-point values, in row-major order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Matrix {
    rows: usize,
    columns: usize,
    values: Vec<Fix16>,
}

impl Matrix {
    /// A zeroed matrix.
    #[must_use]
    pub fn new(rows: usize, columns: usize) -> Self {
        Self {
            rows,
            columns,
            values: vec![Fix16::ZERO; rows * columns],
        }
    }

    /// A matrix from its values, row by row. Returns `None` if there are not
    /// exactly `rows * columns` of them.
    #[must_use]
    pub fn from_values(rows: usize, columns: usize, values: Vec<Fix16>) -> Option<Self> {
        (values.len() == rows * columns).then_some(Self {
            rows,
            columns,
            values,
        })
    }

    /// The identity matrix of a given size.
    #[must_use]
    pub fn identity(size: usize) -> Self {
        let mut result = Self::new(size, size);
        for i in 0..size {
            result.set(i, i, Fix16::ONE);
        }
        result
    }

    /// How many rows.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// How many columns.
    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }

    /// The value at a position.
    ///
    /// # Panics
    ///
    /// If the position is outside the matrix.
    #[must_use]
    pub fn get(&self, row: usize, column: usize) -> Fix16 {
        assert!(
            row < self.rows && column < self.columns,
            "matrix index out of bounds"
        );

        self.values[row * self.columns + column]
    }

    /// Writes a value.
    ///
    /// # Panics
    ///
    /// If the position is outside the matrix.
    pub fn set(&mut self, row: usize, column: usize, value: Fix16) {
        assert!(
            row < self.rows && column < self.columns,
            "matrix index out of bounds"
        );

        self.values[row * self.columns + column] = value;
    }

    /// The transpose.
    #[must_use]
    pub fn transpose(&self) -> Self {
        let mut result = Self::new(self.columns, self.rows);
        for row in 0..result.rows {
            for column in 0..result.columns {
                result.set(row, column, self.get(column, row));
            }
        }
        result
    }

    /// The matrix product, or `None` unless the inner dimensions agree.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Option<Self> {
        if self.columns != other.rows {
            return None;
        }

        let mut result = Self::new(self.rows, other.columns);
        for row in 0..self.rows {
            for column in 0..other.columns {
                let mut sum = Fix16::ZERO;
                for k in 0..self.columns {
                    sum += self.get(row, k) * other.get(k, column);
                }
                result.set(row, column, sum);
            }
        }
        Some(result)
    }

    /// Element-wise sum, or `None` unless the shapes match.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        self.zip(other, |left, right| left + right)
    }

    /// Element-wise difference, or `None` unless the shapes match.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Option<Self> {
        self.zip(other, |left, right| left - right)
    }

    /// Every element multiplied by a constant.
    #[must_use]
    pub fn scale(&self, k: Fix16) -> Self {
        Self {
            rows: self.rows,
            columns: self.columns,
            values: self.values.iter().map(|&value| k * value).collect(),
        }
    }

    /// The inverse of a 2x2 matrix, or `None` for any other shape. A singular
    /// matrix divides by zero, which in fixed point saturates rather than
    /// trapping, so check the determinant if that matters.
    #[must_use]
    pub fn invert(&self) -> Option<Self> {
        if self.rows != 2 || self.columns != 2 {
            return None;
        }

        let determinant = self.get(0, 0) * self.get(1, 1) - self.get(1, 0) * self.get(0, 1);

        let mut result = Self::new(2, 2);
        result.set(0, 0, self.get(1, 1) / determinant);
        result.set(1, 1, self.get(0, 0) / determinant);
        result.set(0, 1, -(self.get(0, 1) / determinant));
        result.set(1, 0, -(self.get(1, 0) / determinant));

        Some(result)
    }

    fn zip(&self, other: &Self, f: impl Fn(Fix16, Fix16) -> Fix16) -> Option<Self> {
        if self.rows != other.rows || self.columns != other.columns {
            return None;
        }

        Some(Self {
            rows: self.rows,
            columns: self.columns,
            values: self
                .values
                .iter()
                .zip(&other.values)
                .map(|(&left, &right)| f(left, right))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(rows: usize, columns: usize, values: &[i32]) -> Matrix {
        Matrix::from_values(
            rows,
            columns,
            values.iter().map(|&v| Fix16::from_int(v)).collect(),
        )
        .unwrap()
    }

    #[test]
    fn multiplication_follows_the_inner_dimension() {
        let left = matrix(2, 3, &[1, 2, 3, 4, 5, 6]);
        let right = matrix(3, 2, &[7, 8, 9, 10, 11, 12]);
        let product = left.mul(&right).unwrap();

        assert_eq!(product.rows(), 2);
        assert_eq!(product.columns(), 2);
        assert_eq!(product.get(0, 0), Fix16::from_int(58));
        assert_eq!(product.get(0, 1), Fix16::from_int(64));
        assert_eq!(product.get(1, 0), Fix16::from_int(139));
        assert_eq!(product.get(1, 1), Fix16::from_int(154));
    }

    #[test]
    fn mismatched_shapes_are_refused() {
        let left = matrix(2, 3, &[1, 2, 3, 4, 5, 6]);
        let right = matrix(2, 3, &[1, 2, 3, 4, 5, 6]);
        assert!(left.mul(&right).is_none());
        assert!(left.add(&matrix(3, 2, &[1, 2, 3, 4, 5, 6])).is_none());
        assert!(left.add(&right).is_some());
    }

    #[test]
    fn transposing_twice_is_the_identity() {
        let original = matrix(2, 3, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(original.transpose().transpose(), original);
        assert_eq!(original.transpose().rows(), 3);
    }

    #[test]
    fn adding_and_subtracting_are_element_wise() {
        let left = matrix(2, 2, &[1, 2, 3, 4]);
        let right = matrix(2, 2, &[10, 20, 30, 40]);

        assert_eq!(left.add(&right).unwrap(), matrix(2, 2, &[11, 22, 33, 44]));
        assert_eq!(right.sub(&left).unwrap(), matrix(2, 2, &[9, 18, 27, 36]));
    }

    #[test]
    fn scaling_multiplies_every_element() {
        let scaled = matrix(1, 3, &[1, 2, 3]).scale(Fix16::from_int(3));
        assert_eq!(scaled, matrix(1, 3, &[3, 6, 9]));
    }

    #[test]
    fn an_inverse_undoes_its_matrix() {
        let original = matrix(2, 2, &[4, 7, 2, 6]);
        let product = original.mul(&original.invert().unwrap()).unwrap();

        // Fixed point will not land on the identity exactly.
        for row in 0..2 {
            for column in 0..2 {
                let expected = if row == column { 1.0 } else { 0.0 };
                assert!((product.get(row, column).to_f64() - expected).abs() < 0.001);
            }
        }
    }

    #[test]
    fn only_two_by_two_matrices_invert() {
        assert!(
            matrix(3, 3, &[1, 0, 0, 0, 1, 0, 0, 0, 1])
                .invert()
                .is_none()
        );
    }

    #[test]
    fn the_identity_leaves_a_matrix_alone() {
        let original = matrix(2, 2, &[1, 2, 3, 4]);
        assert_eq!(original.mul(&Matrix::identity(2)).unwrap(), original);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn reading_past_the_edge_panics() {
        let _ = matrix(2, 2, &[1, 2, 3, 4]).get(2, 0);
    }
}
