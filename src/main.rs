use rayon::prelude::*;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Args {
    original: PathBuf,
    upscaled: PathBuf,
    alphafixed: PathBuf,
}

fn main() {
    match run() {
        Ok(_) => (),
        Err(e) => eprintln!("{e:?}"),
    }
}

fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let args = &args;

    let infiles = walkdir::WalkDir::new(&args.original)
        .max_open(12)
        .into_iter()
        .filter_map(|entry| -> Option<anyhow::Result<PathBuf>> {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => return Some(Err(err.into())),
            };

            if entry.file_type().is_file() {
                Some(Ok(entry.into_path()))
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    infiles
        .par_iter()
        .map(|orig_path| {
            let relative = orig_path.strip_prefix(&args.original).unwrap();
            (
                orig_path,
                args.upscaled.join(relative),
                args.alphafixed.join(relative),
            )
        })
        .try_for_each(
            |(orig_path, upscaled_path, alphafixed_path)| -> anyhow::Result<()> {
                let orig_file = std::fs::OpenOptions::new().read(true).open(orig_path)?;
                let orig_file = std::io::BufReader::new(orig_file);
                let orig = image::load(orig_file, image::ImageFormat::from_path(orig_path)?)?;

                std::fs::create_dir_all(alphafixed_path.parent().unwrap())?;
                let upscaled_file = std::fs::OpenOptions::new()
                    .read(true)
                    .open(&upscaled_path)?;
                let upscaled_file = std::io::BufReader::new(upscaled_file);
                let mut upscaled =
                    image::load(upscaled_file, image::ImageFormat::from_path(orig_path)?)?;

                // Get the original image
                let alpha = orig.as_rgba8().unwrap();

                if !alpha.pixels().any(|x| x.0[3] != 255) {
                    std::fs::copy(upscaled_path, alphafixed_path)?;
                    return Ok(());
                }

                // Upscale the original image using a plain gaussian filter
                let width = upscaled.width();
                let height = upscaled.height();
                let new_alpha = image::imageops::resize(
                    alpha,
                    width,
                    height,
                    image::imageops::FilterType::Lanczos3,
                );
                let w = upscaled.width() as f32;
                let h = upscaled.height() as f32;
                let m = 2.0;
                let proj = imageproc::geometric_transformations::Projection::from_control_points(
                    [(0., 0.), (w, 0.), (0., h), (w, h)],
                    [(m, m), (w - m, m), (m, h - m), (w - m, h - m)],
                )
                .unwrap();
                let new_alpha = imageproc::geometric_transformations::warp(
                    &new_alpha,
                    &proj,
                    imageproc::geometric_transformations::Interpolation::Bicubic,
                    image::Rgba([0, 0, 0, 0]),
                );
                let mut new_alpha = image::imageops::blur(&new_alpha, 3.0);
                let m = m + 2.0;
                let proj = imageproc::geometric_transformations::Projection::from_control_points(
                    [(0., 0.), (w, 0.), (0., h), (w, h)],
                    [(-m, -m), (w + m, -m), (-m, h + m), (w + m, h + m)],
                )
                .unwrap();
                let mut new_alpha = imageproc::geometric_transformations::warp(
                    &new_alpha,
                    &proj,
                    imageproc::geometric_transformations::Interpolation::Bicubic,
                    image::Rgba([0, 0, 0, 0]),
                );
                new_alpha.pixels_mut().for_each(|pixel| {
                    if pixel.0[3] < 128 {
                        pixel.0[3] = 0;
                    } else {
                        pixel.0[3] = 255;
                    }
                });
                let mut new_alpha = image::imageops::blur(&new_alpha, 0.75);

                // Get the AI upscaled image
                let upscaled_rgba8 = upscaled.as_mut_rgba8().unwrap();
                upscaled_rgba8
                    .pixels_mut()
                    .zip(new_alpha.pixels())
                    // Update the alpha channel of the AI upscaled image with the plain-gaussian upscaled alpha
                    .for_each(|(pixel, gaussian_pixel)| {
                        if pixel.0[3] < 200 {
                            pixel.0 = gaussian_pixel.0;
                        } else if pixel.0[3] < gaussian_pixel.0[3] {
                            pixel.0[3] = gaussian_pixel.0[3];
                        }
                    });

                // Save the modified image
                upscaled_rgba8.save(alphafixed_path)?;

                Ok(())
            },
        )?;

    Ok(())
}
