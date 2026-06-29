use libm::erf;
use log::trace;
use std::error::Error;
use std::f64::{
    consts::{SQRT_2, TAU},
    EPSILON, NAN,
};
use std::fmt::{Display, Formatter};

// An error struct representing the class of response errors.
#[non_exhaustive]
#[derive(Debug)]
pub enum MathError {
    // comment string, best estimate, error, max_iter
    NonConvergeance(String, f64, f64, usize),
}

impl Display for MathError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self {
            MathError::NonConvergeance(comment, best, err, n) => write!(
                f,
                "{}: convergeance failed with best estimate {}, error {} after {} iterations",
                comment, best, err, n
            ),
        }
    }
}

impl Error for MathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self {
            MathError::NonConvergeance(_, _, _, _) => None,
        }
    }
}

pub fn norm_pdf(x: f64) -> f64 {
    1.0 / TAU.sqrt() * (-0.5 * (x * x)).exp()
}

pub fn norm_cdf(x: f64) -> f64 {
    erf(x / SQRT_2) * 0.5 + 0.5
}

/// Newton-Raphson unvariate root
///
/// An f64 version of the Newton-Raphson method for find a root of a univariate
/// function with a known derivative.
///
/// # Arguments
/// * `function` - a univariate function
/// * `derivative` - the derivative of the function
/// * `x0` - the initial guess
/// * `max_err` - the maximal error
/// * `max_iter` - the maximal number of iterations
///
/// # Return Value
/// * `Result(f64,f64)` - Either Ok(root) or Err(best guess)
///
/// # Examples
///  ```
/// use bitql::mathutils::root_nr;
///
/// // The function x^3 + x^2  - 4
/// fn f(x: f64) -> f64 {
///     x.powi(3) + x.powi(2) - 4.0
/// }
///
/// // The derivative of f (3x^2 + 2x)
/// fn fd(x: f64) -> f64 {
///     (3.0 * x.powi(2)) + (2.0 * x)
/// }
///
/// let result = root_nr(&f, &fd,
///                        10.0,   // Starting guess
///                        0.0001,     // Precision
///                        1000       // Iterations
///                       ).unwrap();
/// let actual = 1.31459; // The correct answer
///
/// let difference = (result - actual).abs();
///
/// assert!(difference < 0.0001);
/// ```
pub fn root_nr(
    function: &dyn Fn(f64) -> f64,
    derivative: &dyn Fn(f64) -> f64,
    x0: f64,
    max_err: f64,
    max_iter: usize,
) -> Result<f64, MathError> {
    let mut current_x: f64 = x0;
    let mut next_x: f64;

    let mut deviation: f64 = std::f64::MAX;
    for _ in 0..max_iter {
        deviation = function(current_x) / derivative(current_x);
        next_x = current_x - deviation;
        //dbg!(deviation);
        if deviation.abs() <= max_err {
            return Ok(next_x);
        }
        current_x = next_x;
        //dbg!(current_x);
    }
    let ret = MathError::NonConvergeance(
        "Newton Raphston failed ".to_owned(),
        current_x,
        deviation,
        max_iter,
    );
    trace!("{ret}");
    Err(ret)
}

// Numerical Recipes in C by Press, Teukolsky, Vetterling and Flannery. p301ff
pub fn root_brent(
    x1: f64,
    x2: f64,
    func: &dyn Fn(f64) -> f64,
    max_err: f64,
    max_iter: usize,
) -> Result<f64, MathError> {
    let mut a = x1;
    let mut b = x2;
    let mut c = x2;

    let mut fa = func(a);
    let mut fb = func(b);
    if (fa > 0.0 && fb > 0.0) || (fa < 0.0 && fb < 0.0) {
        return Err(MathError::NonConvergeance(
            format!(
                "Brent failed: starting points {} {} should enclose 0.",
                x1, x2
            ),
            NAN,
            NAN,
            0,
        ));
    }

    let mut d = NAN;
    let mut e = NAN;
    let mut q;
    let mut r;
    let mut p;

    let mut fc = fb;
    for _ in 0..max_iter {
        if (fb > 0.0 && fc > 0.0) || (fb < 0.0 && fc < 0.0) {
            // rename a, b, c and adjust bounding interval d
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        // convergence check
        let tol1 = 2.0 * EPSILON * b.abs() + 0.5 * max_err;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return Ok(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            // attempt inverse quadratic interpolation
            let s = fb / fa;
            if a == c {
                p = 2.0 * xm * s;
                q = 1.0 - s;
            } else {
                q = fa / fc;
                r = fb / fc;
                p = s * (2.0 * xm * q * (q - r) - (b - a) * (r - 1.0));
                q = (q - 1.0) * (r - 1.0) * (s - 1.0);
            }
            // check whether in bounds
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            let min1 = 3.0 * xm * q * (tol1 - q).abs();
            let min2 = (e * q).abs();
            if 2.0 * p < min1.min(min2) {
                // accept interpolation
                e = d;
                d = p / q;
            } else {
                // interpolation failed, use bisection
                d = xm;
                e = d;
            }
        } else {
            // bounds decreasing too slowly, use bisection
            d = xm;
            e = d;
        }

        // move last best guess to a
        a = b;
        fa = fb;
        // evaluate new trial root
        if d.abs() > tol1 {
            b += d;
        } else {
            b += tol1.abs() * xm.signum();
        }
        fb = func(b);
    }
    let ret = MathError::NonConvergeance("Brent failed ".to_owned(), b, max_err, max_iter);
    trace!("{ret}");
    Err(ret)
}

// TESTS

#[cfg(test)]
mod tests {
    use crate::mathutils::root_nr;
    #[test]
    fn check_root_nr_1() {
        let tol_digits = 12;
        let eps = f64::powi(10.0f64, -tol_digits);
        fn f(x: f64) -> f64 {
            x.powi(3) + x.powi(2) - 4.0
        }
        fn fd(x: f64) -> f64 {
            (3.0 * x.powi(2)) + (2.0 * x)
        }
        let result = root_nr(&f, &fd, 10.0, eps, 1000).unwrap();
        let actual: f64 = 1.314_596_212_276_752;
        assert!((result - actual).abs() < 1e-12);
    }
    #[test]
    fn check_root_nr_2() {
        let tol_digits = 12;
        let eps = f64::powi(10.0f64, -tol_digits);
        fn f(x: f64) -> f64 {
            x.powi(3) + x.powi(2) - 4.0
        }
        fn fd(x: f64) -> f64 {
            (3.0 * x.powi(2)) + (2.0 * x)
        }
        let result = root_nr(&f, &fd, 10.0, eps, 8);
        assert!(result.is_err());
    }
}
