#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use crate::image::ToRGB;
use crate::image::RGBAPLU;
use crate::image::RGBLU;
use imgref::*;
#[cfg(not(feature = "threads"))]
use crate::lieon as rayon;
use rayon::prelude::*;

const D65x: f32 = 0.9505;
const D65z: f32 = 1.089;

pub type GBitmap = ImgVec<f32>;
pub(crate) trait ToLAB {
    fn to_lab(&self) -> (f32, f32, f32);
}

#[inline(always)]
fn fma_matrix(r: f32, rx: f32, g: f32, gx: f32, b: f32, bx: f32) -> f32 {
    b.mul_add(bx, g.mul_add(gx, r * rx))
}

impl ToLAB for RGBLU {
    fn to_lab(&self) -> (f32, f32, f32) {
        let fx = fma_matrix(self.r, 0.4124, self.g, 0.3576, self.b, 0.1805) / D65x;
        let fy = fma_matrix(self.r, 0.2126, self.g, 0.7152, self.b, 0.0722); // D65y is 1.0
        let fz = fma_matrix(self.r, 0.0193, self.g, 0.1192, self.b, 0.9505) / D65z;

        let epsilon: f32 = 216. / 24389.;
        let k = 24389. / (27. * 116.); // http://www.brucelindbloom.com/LContinuity.html
        let X = if fx > epsilon { fx.cbrt() - 16. / 116. } else { k * fx };
        let Y = if fy > epsilon { fy.cbrt() - 16. / 116. } else { k * fy };
        let Z = if fz > epsilon { fz.cbrt() - 16. / 116. } else { k * fz };

        let lab = (
            (Y * 1.05f32), // 1.05 instead of 1.16 to boost color importance without pushing colors outside of 1.0 range
            (500.0 / 220.0f32).mul_add(X - Y, 86.2 / 220.0f32), /* 86 is a fudge to make the value positive */
            (200.0 / 220.0f32).mul_add(Y - Z, 107.9 / 220.0f32), /* 107 is a fudge to make the value positive */
        );
        debug_assert!(lab.0 <= 1.0 && lab.1 <= 1.0 && lab.2 <= 1.0);
        lab
    }
}

/// Convert image to L\*a\*b\* planar
///
/// It should return 1 (gray) or 3 (color) planes.
pub trait ToLABBitmap {
    fn to_lab(&self) -> Vec<GBitmap>;
}

impl ToLABBitmap for ImgVec<RGBAPLU> {
    #[inline(always)]
    fn to_lab(&self) -> Vec<GBitmap> {
        self.as_ref().to_lab()
    }
}

impl ToLABBitmap for ImgVec<RGBLU> {
    #[inline(always)]
    fn to_lab(&self) -> Vec<GBitmap> {
        self.as_ref().to_lab()
    }
}

impl ToLABBitmap for GBitmap {
    #[inline(never)]
    fn to_lab(&self) -> Vec<GBitmap> {
        let width = self.width();
        assert!(width > 0);
        let height = self.height();
        let area = width * height;
        let mut out = Vec::with_capacity(area);

        // For output width == stride
        out.spare_capacity_mut().par_chunks_exact_mut(width).take(height).enumerate().for_each(|(y, out_row)| {
            let in_row = &self.rows().nth(y).unwrap()[0..width];
            let out_row = &mut out_row[0..width];
            let epsilon: f32 = 216. / 24389.;
            for x in 0..width {
                let fy = in_row[x];
                // http://www.brucelindbloom.com/LContinuity.html
                let Y = if fy > epsilon { fy.cbrt() - 16. / 116. } else { ((24389. / 27.) / 116.) * fy };
                out_row[x].write(Y * 1.16);
            }
        });

        unsafe { out.set_len(area) };
        vec![Img::new(out, width, height)]
    }
}

#[inline(never)]
fn rgb_to_lab<T: Copy + Sync + Send + 'static, F>(img: ImgRef<'_, T>, cb: F) -> Vec<GBitmap>
    where F: Fn(T, usize) -> (f32, f32, f32) + Sync + Send + 'static
{
    let width = img.width();
    assert!(width > 0);
    let height = img.height();
    let area = width * height;

    let mut out_l = Vec::with_capacity(area);
    let mut out_a = Vec::with_capacity(area);
    let mut out_b = Vec::with_capacity(area);

    // For output width == stride
    out_l.spare_capacity_mut().par_chunks_exact_mut(width).take(height).zip(
        out_a.spare_capacity_mut().par_chunks_exact_mut(width).take(height).zip(
            out_b.spare_capacity_mut().par_chunks_exact_mut(width).take(height))
    ).enumerate()
    .for_each(|(y, (l_row, (a_row, b_row)))| {
        let in_row = &img.rows().nth(y).unwrap()[0..width];
        let l_row = &mut l_row[0..width];
        let a_row = &mut a_row[0..width];
        let b_row = &mut b_row[0..width];
        for x in 0..width {
            let n = (x+11) ^ (y+11);
            let (l,a,b) = cb(in_row[x], n);
            l_row[x].write(l);
            a_row[x].write(a);
            b_row[x].write(b);
        }
    });

    unsafe { out_l.set_len(area) };
    unsafe { out_a.set_len(area) };
    unsafe { out_b.set_len(area) };

    vec![
        Img::new(out_l, width, height),
        Img::new(out_a, width, height),
        Img::new(out_b, width, height),
    ]
}

impl<'a> ToLABBitmap for ImgRef<'a, RGBAPLU> {
    #[inline]
    fn to_lab(&self) -> Vec<GBitmap> {
        rgb_to_lab(*self, |px, n|{
            px.to_rgb(n).to_lab()
        })
    }
}

impl<'a> ToLABBitmap for ImgRef<'a, RGBLU> {
    #[inline]
    fn to_lab(&self) -> Vec<GBitmap> {
        rgb_to_lab(*self, |px, _n|{
            px.to_lab()
        })
    }
}
