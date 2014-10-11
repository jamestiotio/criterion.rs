use stats::ConfidenceInterval;
use stats::outliers::Outliers;
use stats::regression::Slope ;
use stats::{Distribution, Sample};
use std::fmt::Show;
use std::io::Command;
use std::io::fs::PathExtensions;
use time;

use estimate::{
    Distributions,
    Estimate,
    Estimates,
    Mean,
    Median,
    MedianAbsDev,
    StdDev,
    mod,
};
use format;
use fs;
use plot;
use program::Program;
use report;
use routine::{Function, Routine};
use {Bencher, Criterion};

macro_rules! elapsed {
    ($msg:expr, $block:expr) => ({
        let start = time::precise_time_ns();
        let out = $block;
        let stop = time::precise_time_ns();

        info!("{} took {}", $msg, format::time((stop - start) as f64));

        out
    })
}

mod compare;

pub fn summarize(id: &str, criterion: &Criterion) {
    if criterion.plotting.is_enabled() {
        print!("Summarizing results of {}... ", id);
        plot::summarize(id);
        println!("DONE\n");
    } else {
        println!("Plotting disabled, skipping summarization");
    }
}

pub fn function(id: &str, f: |&mut Bencher|:'static, criterion: &Criterion) {
    common(id, &mut Function(f), criterion);

    println!("");
}

pub fn function_with_inputs<I: Show>(
    id: &str,
    f: |&mut Bencher, &I|:'static,
    inputs: &[I],
    criterion: &Criterion,
) {
    for input in inputs.iter() {
        let id = format!("{}/{}", id, input);

        common(id.as_slice(), &mut Function(|b| f(b, input)), criterion);
    }

    summarize(id, criterion);
}

pub fn program(id: &str, prog: &Command, criterion: &Criterion) {
    common(id, &mut Program::spawn(prog), criterion);

    println!("");
}

pub fn program_with_inputs<I: Show>(
    id: &str,
    prog: &Command,
    inputs: &[I],
    criterion: &Criterion,
) {
    for input in inputs.iter() {
        let id = format!("{}/{}", id, input);

        program(id.as_slice(), prog.clone().arg(format!("{}", input)), criterion);
    }

    summarize(id, criterion);
}

// Common analysis procedure
fn common(id: &str, routine: &mut Routine, criterion: &Criterion) {
    println!("Benchmarking {}", id);

    let pairs = routine.sample(criterion);

    rename_new_dir_to_base(id);

    let pairs_f64 = pairs.iter().map(|&(iters, elapsed)| {
        (iters as f64, elapsed as f64)
    }).collect::<Vec<(f64, f64)>>();

    let times = pairs.iter().map(|&(iters, elapsed)| {
        elapsed as f64 / iters as f64
    }).collect::<Vec<f64>>();
    let times = times[];

    fs::mkdirp(&Path::new(format!(".criterion/{}/new", id)));

    let outliers = outliers(id, times);
    if criterion.plotting.is_enabled() {
        elapsed!(
            "Plotting the estimated sample PDF",
            plot::pdf(times, &outliers, id));
    }
    let (distribution, slope) = regression(id, pairs_f64[], criterion);
    let (mut distributions, mut estimates) = estimates(times, criterion);

    estimates.insert(estimate::Slope, slope);
    distributions.insert(estimate::Slope, distribution);

    if criterion.plotting.is_enabled() {
        elapsed!(
            "Plotting the distribution of the absolute statistics",
            plot::abs_distributions(
                &distributions,
                &estimates,
                id));
    }

    fs::save(&pairs, &Path::new(format!(".criterion/{}/new/sample.json", id)));
    fs::save(&outliers, &Path::new(format!(".criterion/{}/new/outliers.json", id)));
    fs::save(&estimates, &Path::new(format!(".criterion/{}/new/estimates.json", id)));

    if base_dir_exists(id) {
        compare::common(id, pairs_f64[], times, &estimates, criterion);
    }
}

fn base_dir_exists(id: &str) -> bool {
    Path::new(format!(".criterion/{}/base", id)).exists()
}
// Performs a simple linear regression on the sample
fn regression(
    id: &str,
    pairs: &[(f64, f64)],
    criterion: &Criterion,
) -> (Distribution<f64>, Estimate) {
    fn slr(sample: &[(f64, f64)]) -> f64 {
        Slope::fit(sample).0
    }

    let cl = criterion.confidence_level;

    println!("> Performing linear regression");

    let sample = Sample::new(pairs);
    let distribution = elapsed!(
        "Bootstrapped linear regression",
        sample.bootstrap(slr, criterion.nresamples));

    let point = Slope::fit(pairs);
    let ConfidenceInterval { lower_bound: lb, upper_bound: ub, .. } =
        distribution.confidence_interval(criterion.confidence_level);
    let se = distribution.standard_error();

    let (lb_, ub_) = (Slope(lb), Slope(ub));

    report::regression(pairs, (&lb_, &ub_));

    if criterion.plotting.is_enabled() {
        elapsed!(
            "Plotting linear regression",
            plot::regression(
                pairs,
                &point,
                (&lb_, &ub_),
                id));
    }

    (distribution, Estimate {
        confidence_interval: ConfidenceInterval {
            confidence_level: cl,
            lower_bound: lb,
            upper_bound: ub,
        },
        point_estimate: point.0,
        standard_error: se,
    })
}

// Classifies the outliers in the sample
fn outliers(id: &str, times: &[f64]) -> Outliers<f64> {
    let outliers = Outliers::classify(times);

    report::outliers(&outliers);
    // FIXME Remove labels before saving
    fs::save(&outliers, &Path::new(format!(".criterion/{}/new/outliers.json", id)));

    outliers
}

// Estimates the statistics of the population from the sample
fn estimates(
    times: &[f64],
    criterion: &Criterion,
) -> (Distributions, Estimates) {
    fn stats(a: &[f64]) -> (f64, f64, f64, f64) {
        (Mean.abs_fn()(a), Median.abs_fn()(a), MedianAbsDev.abs_fn()(a), StdDev.abs_fn()(a))
    }

    let cl = criterion.confidence_level;
    let nresamples = criterion.nresamples;

    let points = {
        let (a, b, c, d) = stats(times);

        [a, b, c, d]
    };

    println!("> Estimating the statistics of the sample");
    let sample = Sample::new(times);
    let distributions = {
        let (a, b, c, d) = elapsed!(
        "Bootstrapping the absolute statistics",
        sample.bootstrap(stats, nresamples)).split4();

        vec![a, b, c, d]
    };
    let distributions: Distributions = [Mean, Median, MedianAbsDev, StdDev].iter().map(|&x| {
        x
    }).zip(distributions.into_iter()).collect();
    let estimates = Estimate::new(&distributions, points[], cl);

    report::abs(&estimates);

    (distributions, estimates)
}

fn rename_new_dir_to_base(id: &str) {
    let root_dir = Path::new(".criterion").join(id);
    let base_dir = root_dir.join("base");
    let new_dir = root_dir.join("new");

    if base_dir.exists() { fs::rmrf(&base_dir) }
    if new_dir.exists() { fs::mv(&new_dir, &base_dir) };
}