#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
/*
 * © 2011-2017 Kornel Lesiński. All rights reserved.
 *
 * This file is part of DSSIM.
 *
 * DSSIM is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License
 * as published by the Free Software Foundation, either version 3
 * of the License, or (at your option) any later version.
 *
 * DSSIM is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the license along with DSSIM.
 * If not, see <http://www.gnu.org/licenses/agpl.txt>.
 */

use crate::blur;
use crate::image::*;
use crate::linear::ToRGBAPLU;
pub use crate::tolab::ToLABBitmap;
pub use crate::val::Dssim as Val;
use imgref::*;
use itertools::multizip;
#[cfg(not(feature = "threads"))]
use crate::lieon as rayon;
use rayon::prelude::*;
use rgb::{RGB, RGBA};
use std::borrow::Borrow;
use std::mem::MaybeUninit;
use std::ops;
use std::ops::Deref;
use std::sync::Arc;

trait Channable<T, I> {
    fn img1_img2_blur(&self, modified: &Self, tmp: &mut [MaybeUninit<I>]) -> Vec<T>;
}

#[derive(Clone)]
struct DssimChan<T> {
    pub width: usize,
    pub height: usize,
    pub img: Option<ImgVec<T>>,
    pub mu: Vec<T>,
    pub img_sq_blur: Vec<T>,
    pub is_chroma: bool,
}

/// Configuration for the comparison
#[derive(Clone, Debug)]
pub struct Dssim {
    scale_weights: Vec<f64>,
    save_maps_scales: u8,
}

#[derive(Clone)]
struct DssimChanScale<T> {
    chan: Vec<DssimChan<T>>,
}

/// Abstract wrapper for images. See [`Dssim::create_image()`]
#[derive(Clone)]
pub struct DssimImage<T> {
    scale: Vec<DssimChanScale<T>>,
}

impl<T> DssimImage<T> {
    #[inline]
    #[must_use]
    pub fn width(&self) -> usize {
        self.scale[0].chan[0].width
    }

    #[inline]
    #[must_use]
    pub fn height(&self) -> usize {
        self.scale[0].chan[0].height
    }
}

// Weighed scales are inspired by the IW-SSIM, but details of the algorithm and weights are different
const DEFAULT_WEIGHTS: [f64; 5] = [0.028, 0.197, 0.322, 0.298, 0.155];

/// Detailed comparison result
#[derive(Clone)]
pub struct SsimMap {
    /// SSIM scores
    pub map: ImgVec<f32>,
    /// Average SSIM (not DSSIM)
    pub ssim: f64,
}

/// Create new context for a comparison
#[must_use]
pub fn new() -> Dssim {
    Dssim::new()
}

impl DssimChan<f32> {
    pub fn new(bitmap: ImgVec<f32>, is_chroma: bool) -> Self {
        debug_assert!(bitmap.pixels().all(|i| i.is_finite() && i >= 0.0 && i <= 1.0));

        DssimChan {
            width: bitmap.width(),
            height: bitmap.height(),
            mu: Vec::new(),
            img: Some(bitmap),
            img_sq_blur: Vec::new(),
            is_chroma,
        }
    }
}

impl DssimChan<f32> {
    fn preprocess(&mut self, tmp: &mut [MaybeUninit<f32>]) {
        let width = self.width;
        let height = self.height;
        assert!(width > 0);
        assert!(height > 0);

        let img = self.img.as_mut().unwrap();
        debug_assert_eq!(width * height, img.pixels().count());
        debug_assert!(img.pixels().all(|i| i.is_finite()));

        if self.is_chroma {
            blur::blur_in_place(img.as_mut(), tmp);
        }
        let (mu, _, _) = blur::blur(img.as_ref(), tmp).into_contiguous_buf();
        self.mu = mu;

        self.img_sq_blur = img.pixels().map(|i| {
            debug_assert!(i <= 1.0 && i >= 0.0);
            i * i
        }).collect();
        blur::blur_in_place(ImgRefMut::new(&mut self.img_sq_blur[..], width, height), tmp);
    }
}

impl Channable<LAB, f32> for [DssimChan<f32>] {
    fn img1_img2_blur(&self, modified: &Self, tmp32: &mut [MaybeUninit<f32>]) -> Vec<LAB> {

        let blurred:Vec<_> = self.iter().zip(modified.iter()).map(|(o,m)|{
            o.img1_img2_blur(m, tmp32)
        }).collect();

        return multizip((blurred[0].iter().copied(), blurred[1].iter().copied(), blurred[2].iter().copied())).map(|(l,a,b)| {
            LAB {l,a,b}
        }).collect();
    }
}

impl Channable<f32, f32> for DssimChan<f32> {
    fn img1_img2_blur(&self, modified: &Self, tmp32: &mut [MaybeUninit<f32>]) -> Vec<f32> {
        let modified_img = modified.img.as_ref().unwrap();
        let width = modified_img.width();
        let height = modified_img.height();

        let mut out = Vec::with_capacity(width * height);

        for (row1, row2) in self.img.as_ref().unwrap().rows().zip(modified_img.rows()) {
            debug_assert_eq!(width, row1.len());
            debug_assert_eq!(width, row2.len());
            let row1 = &row1[0..width];
            let row2 = &row2[0..width];
            for (px1, px2) in row1.iter().copied().zip(row2.iter().copied()) {
                debug_assert!(px1 <= 1.0 && px1 >= 0.0);
                debug_assert!(px2 <= 1.0 && px2 >= 0.0);
                out.push(px1 * px2);
            }
        }

        debug_assert_eq!(out.len(), width * height);
        blur::blur_in_place(ImgRefMut::new(&mut out, width, height), tmp32);
        out
    }
}

impl Dssim {
    /// Create new context for comparisons
    #[must_use]
    pub fn new() -> Dssim {
        Dssim {
            scale_weights: DEFAULT_WEIGHTS[..].to_owned(),
            save_maps_scales: 0,
        }
    }

    /// Set how many scales will be used, and weights of each scale
    pub fn set_scales(&mut self, scales: &[f64]) {
        self.scale_weights = scales.to_vec();
    }

    /// Set how many scales will be kept for saving
    pub fn set_save_ssim_maps(&mut self, num_scales: u8) {
        self.save_maps_scales = num_scales;
    }

    /// Create image from an array of RGBA pixels (sRGB, non-premultiplied, alpha last).
    ///
    /// If you have a slice of `u8`, then see `rgb` crate's `as_rgba()`.
    #[must_use] pub fn create_image_rgba(&self, bitmap: &[RGBA<u8>], width: usize, height: usize) -> Option<DssimImage<f32>> {
        if width * height < bitmap.len() {
            return None;
        }
        let img = ImgVec::new(bitmap.to_rgbaplu(), width, height);
        self.create_image(&img)
    }

    /// Create image from an array of packed RGB pixels (sRGB).
    ///
    /// If you have a slice of `u8`, then see `rgb` crate's `as_rgb()`.
    #[must_use] pub fn create_image_rgb(&self, bitmap: &[RGB<u8>], width: usize, height: usize) -> Option<DssimImage<f32>> {
        if width * height < bitmap.len() {
            return None;
        }
        let img = ImgVec::new(bitmap.to_rgblu(), width, height);
        self.create_image(&img)
    }

    /// The input image is defined using the `imgref` crate, and the pixel type can be:
    ///
    /// * `ImgVec<RGBAPLU>` — RGBA premultiplied alpha, linear, float scaled to 0..1
    /// * `ImgVec<RGBLU>` — RGBA linear, float scaled to 0..1
    /// * `ImgVec<f32>` — linear light grayscale, float scaled to 0..1
    ///
    /// And there's [`ToRGBAPLU::to_rgbaplu()`][crate::ToRGBAPLU::to_rgbaplu()] trait to convert the input pixels from
    /// `[RGBA<u8>]`, `[RGBA<u16>]`, `[RGB<u8>]`, or `RGB<u16>`. See `lib.rs` for example how it's done.
    ///
    /// You can implement `ToLABBitmap` and `Downsample` traits on your own image type.
    pub fn create_image<InBitmap, OutBitmap>(&self, src_img: &InBitmap) -> Option<DssimImage<f32>>
    where
        InBitmap: ToLABBitmap + Send + Sync + Downsample<Output = OutBitmap>,
        OutBitmap: ToLABBitmap + Send + Sync + Downsample<Output = OutBitmap>,
    {
        let num_scales = self.scale_weights.len();
        let mut scale = Vec::with_capacity(num_scales);
        Self::make_scales_recursive(num_scales, MaybeArc::Borrowed(src_img), &mut scale);
        scale.reverse(); // depth-first made smallest scales first

        Some(DssimImage { scale })
    }

    #[inline(never)]
    fn make_scales_recursive<InBitmap, OutBitmap>(scales_left: usize, image: MaybeArc<'_, InBitmap>, scales: &mut Vec<DssimChanScale<f32>>)
    where
        InBitmap: ToLABBitmap + Send + Sync + Downsample<Output = OutBitmap>,
        OutBitmap: ToLABBitmap + Send + Sync + Downsample<Output = OutBitmap>,
    {
        // Run to_lab and next downsampling in parallel
        let (chan, _) = rayon::join({
            let image = image.clone();
            move || {
                let lab = image.to_lab();
                drop(image); // Free larger RGB image ASAP
                DssimChanScale {
                    chan: lab.into_par_iter().enumerate().map(|(n,l)| {
                        let w = l.width();
                        let h = l.height();
                        let mut ch = DssimChan::new(l, n > 0);

                        let pixels = w * h;
                        let mut tmp = Vec::with_capacity(pixels);
                        ch.preprocess(&mut tmp.spare_capacity_mut()[..pixels]);
                        ch
                    }).collect(),
                }
            }
        }, {
            let scales = &mut *scales;
            move || {
                if scales_left > 0 {
                    let down = image.downsample();
                    drop(image);
                    if let Some(downsampled) = down {
                        Self::make_scales_recursive(scales_left - 1, MaybeArc::Owned(Arc::new(downsampled)), scales);
                    }
                }
            }
        });
        scales.push(chan);
    }

    /// Compare original with another image. See `create_image`
    ///
    /// The `SsimMap`s are returned only if you've enabled them first.
    ///
    /// `Val` is a fancy wrapper for `f64`
    #[inline(never)]
    pub fn compare<M: Borrow<DssimImage<f32>>>(&self, original_image: &DssimImage<f32>, modified_image: M) -> (Val, Vec<SsimMap>) {
        let modified_image = modified_image.borrow();
        let res: Vec<_> = self.scale_weights.as_slice().par_iter().with_min_len(1).cloned().zip(
                        modified_image.scale.as_slice().par_iter().with_min_len(1).zip(
                        original_image.scale.as_slice().par_iter().with_min_len(1))
        ).enumerate().map(|(n, (weight, (modified_image_scale, original_image_scale)))| {
            let scale_width = original_image_scale.chan[0].width;
            let scale_height = original_image_scale.chan[0].height;
            let mut tmp = Vec::with_capacity(scale_width * scale_height);
            let tmp = &mut tmp.spare_capacity_mut()[0 .. scale_width*scale_height];

            let ssim_map = match original_image_scale.chan.len() {
                3 => {
                    let (original_lab, (img1_img2_blur, modified_lab)) = rayon::join(
                    || Self::lab_chan(original_image_scale),
                    || {
                        let img1_img2_blur = original_image_scale.chan.img1_img2_blur(&modified_image_scale.chan, tmp);
                        (img1_img2_blur, Self::lab_chan(modified_image_scale))
                    });

                    Self::compare_scale(&original_lab, &modified_lab, &img1_img2_blur)
                },
                1 => {
                    let img1_img2_blur = original_image_scale.chan[0].img1_img2_blur(&modified_image_scale.chan[0], tmp);
                    Self::compare_scale(&original_image_scale.chan[0], &modified_image_scale.chan[0], &img1_img2_blur)
                },
                _ => panic!(),
            };

            let sum = ssim_map.pixels().fold(0., |sum, i| sum + f64::from(i));
            let len = (ssim_map.width()*ssim_map.height()) as f64;
            let avg = (sum / len).max(0.0).powf((0.5_f64).powf(n as f64));
            let score = 1.0 - (ssim_map.pixels().fold(0., |sum, i| sum + (avg - f64::from(i)).abs()) / len);

            let map = if self.save_maps_scales as usize > n {
                Some(SsimMap {
                    map: ssim_map,
                    ssim: score,
                })
            } else {
                None
            };
            (score, weight, map)
        }).collect();

        let mut ssim_sum = 0.0;
        let mut weight_sum = 0.0;
        let mut ssim_maps = Vec::new();
        for (score, weight, map) in res {
            ssim_sum += score * weight;
            weight_sum += weight;
            if let Some(m) = map {
                ssim_maps.push(m);
            }
        }

        (to_dssim(ssim_sum / weight_sum).into(), ssim_maps)
    }

    fn lab_chan(scale: &DssimChanScale<f32>) -> DssimChan<LAB> {
        let l = &scale.chan[0];
        let a = &scale.chan[1];
        let b = &scale.chan[2];
        assert_eq!(l.width, a.width);
        assert_eq!(b.width, a.width);
        DssimChan {
            img_sq_blur: multizip((l.img_sq_blur.iter().copied(), a.img_sq_blur.iter().copied(), b.img_sq_blur.iter().copied()))
                .map(|(l,a,b)|LAB {l,a,b}).collect(),
            img: if let (Some(l),Some(a),Some(b)) = (&l.img, &a.img, &b.img) {
                let buf = multizip((l.pixels(), a.pixels(), b.pixels())).map(|(l,a,b)|{
                    debug_assert!(l.is_finite() && a.is_finite() && b.is_finite());
                    LAB {l,a,b}
                }).collect();
                Some(ImgVec::new(buf, l.width(), l.height()))
            } else {None},
            mu: multizip((l.mu.iter().copied(), a.mu.iter().copied(), b.mu.iter().copied())).map(|(l,a,b)|LAB {l,a,b}).collect(),
            is_chroma: false,
            width: l.width,
            height: l.height,
        }
    }

    #[inline(never)]
    fn compare_scale<L>(original: &DssimChan<L>, modified: &DssimChan<L>, img1_img2_blur: &[L]) -> ImgVec<f32>
    where
        L: Send + Sync + Clone + Copy + ops::Mul<Output = L> + ops::Sub<Output = L> + 'static,
        f32: From<L>,
    {
        assert_eq!(original.width, modified.width);
        assert_eq!(original.height, modified.height);

        let width = original.width;
        let height = original.height;

        let c1 = 0.01 * 0.01;
        let c2 = 0.03 * 0.03;

        debug_assert_eq!(original.mu.len(), modified.mu.len());
        debug_assert_eq!(original.img_sq_blur.len(), modified.img_sq_blur.len());
        debug_assert_eq!(img1_img2_blur.len(), original.mu.len());
        debug_assert_eq!(img1_img2_blur.len(), original.img_sq_blur.len());

        let mu_iter = original.mu.as_slice().par_iter().with_min_len(1<<10).cloned().zip_eq(modified.mu.as_slice().par_iter().with_min_len(1<<10).cloned());
        let sq_iter = original.img_sq_blur.as_slice().par_iter().with_min_len(1<<10).cloned().zip_eq(modified.img_sq_blur.as_slice().par_iter().with_min_len(1<<10).cloned());
        let map_out = img1_img2_blur.par_iter().with_min_len(1<<10).cloned().zip_eq(mu_iter).zip_eq(sq_iter)
        .map(|((img1_img2_blur, (mu1, mu2)), (img1_sq_blur, img2_sq_blur))| {
            let mu1mu1 = mu1 * mu1;
            let mu1mu2 = mu1 * mu2;
            let mu2mu2 = mu2 * mu2;
            let mu1_sq: f32 = mu1mu1.into();
            let mu2_sq: f32 = mu2mu2.into();
            let mu1_mu2: f32 = mu1mu2.into();
            let sigma1_sq: f32 = (img1_sq_blur - mu1mu1).into();
            let sigma2_sq: f32 = (img2_sq_blur - mu2mu2).into();
            let sigma12: f32 = (img1_img2_blur - mu1mu2).into();

            2.0f32.mul_add(mu1_mu2, c1) * 2.0f32.mul_add(sigma12, c2) /
                       ((mu1_sq + mu2_sq + c1) * (sigma1_sq + sigma2_sq + c2))
        }).collect();

        ImgVec::new(map_out, width, height)
    }
}

fn to_dssim(ssim: f64) -> f64 {
    1.0 / ssim.max(std::f64::EPSILON) - 1.0
}

#[test]
fn png_compare() {
    use crate::linear::*;
    use imgref::*;

    let d = new();
    let file1 = lodepng::decode32_file("../tests/test1-sm.png").unwrap();
    let file2 = lodepng::decode32_file("../tests/test2-sm.png").unwrap();

    let buf1 = &file1.buffer.to_rgbaplu()[..];
    let buf2 = &file2.buffer.to_rgbaplu()[..];
    let img1 = d.create_image(&Img::new(buf1, file1.width, file1.height)).unwrap();
    let img2 = d.create_image(&Img::new(buf2, file2.width, file2.height)).unwrap();

    let (res, _) = d.compare(&img1, img2);
    assert!((0.001 - res).abs() < 0.0005, "res is {res}");

    let img1b = d.create_image(&Img::new(buf1, file1.width, file1.height)).unwrap();
    let (res, _) = d.compare(&img1, img1b);

    assert!(0.000000000000001 > res);
    assert!(res < 0.000000000000001);
    assert_eq!(res, res);

    let sub_img1 = d.create_image(&Img::new(buf1, file1.width, file1.height).sub_image(2,3,44,33)).unwrap();
    let sub_img2 = d.create_image(&Img::new(buf2, file2.width, file2.height).sub_image(17,9,44,33)).unwrap();
    // Test passing second image directly
    let (res, _) = d.compare(&sub_img1, sub_img2);
    assert!(res > 0.1);

    let sub_img1 = d.create_image(&Img::new(buf1, file1.width, file1.height).sub_image(22,8,61,40)).unwrap();
    let sub_img2 = d.create_image(&Img::new(buf2, file2.width, file2.height).sub_image(22,8,61,40)).unwrap();
    // Test passing second image as reference
    let (res, _) = d.compare(&sub_img1, sub_img2);
    assert!(res < 0.01);
}

enum MaybeArc<'a, T> {
    Owned(Arc<T>),
    Borrowed(&'a T),
}

impl<'a, T> Clone for MaybeArc<'a, T> {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(t) => Self::Owned(t.clone()),
            Self::Borrowed(t) => Self::Borrowed(t),
        }
    }
}

impl<'a, T> Deref for MaybeArc<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => t,
        }
    }
}



#[test]
fn poison() {
    let a = RGBAPLU::new(1.,1.,1.,1.);
    let b = RGBAPLU::new(0.,0.,0.,0.);
    let n = 1./0.;
    let n = RGBAPLU::new(n,n,n,n);
    let buf = vec![
      b,a,a,b,n,n,
      a,b,b,a,n,n,
      b,a,a,b,n,
    ];
    let img = ImgVec::new_stride(buf, 4, 3, 6);
    assert!(img.pixels().all(|p| p.r.is_finite() && p.a.is_finite()));
    assert!(img.as_ref().pixels().all(|p| p.g.is_finite() && p.b.is_finite()));

    let d = new();
    let sub_img1 = d.create_image(&img.as_ref()).unwrap();
    let sub_img2 = d.create_image(&img.as_ref()).unwrap();
    let (res, _) = d.compare(&sub_img1, sub_img2);
    assert!(res < 0.000001);
}
