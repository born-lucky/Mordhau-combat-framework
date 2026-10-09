//! The Kismet random nodes the mode Blueprints call (Array_Shuffle, RandomInteger, RandomIntegerInRange), on the
//! world's CRT rand() stream (mordhau_core::ue::CrtRand: the one stream every caller shares, ue_rand.gd header).
//!
//! UE 4.26 engine code, not disassembled (UNCONFIRMED, from the 4.26 source the exe was built from):
//!   UKismetArrayLibrary::GenericArray_Shuffle: for i in 0..=LastIndex { Index = FMath::RandRange(i, LastIndex);
//!     if i != Index swap(i, Index) }
//!   UKismetMathLibrary::RandomInteger(Max) = FMath::RandHelper(Max); RandomIntegerInRange(Min, Max) =
//!     FMath::RandRange(Min, Max) = Min + RandHelper(Max - Min + 1)
//!   FMath::RandHelper(A) = A > 0 ? Min(TruncToInt(FRand() * A), A - 1) : 0, all in float
//!   FMath::FRand = Rand() / (float)RAND_MAX: taken as the game code's inlined form (rand() & 0x7fff) * (1/32767f)
//!     (.rdata 0x1440dfae0, e.g. UBotBehaviorProfile::RerollRandomInstanceValues rva=0x149e910)

use crate::consts::bot::FRAND_SCALE;
use mordhau_core::ue::CrtRand;

/// FMath::FRand (float)
pub fn frand(rng: &mut CrtRand) -> f32 {
    (rng.rand() & 0x7fff) as f32 * FRAND_SCALE as f32
}

/// FMath::RandHelper
pub fn rand_helper(rng: &mut CrtRand, a: i64) -> i64 {
    if a > 0 {
        ((frand(rng) * a as f32) as i64).min(a - 1)
    } else {
        0
    }
}

/// UKismetMathLibrary::RandomInteger
pub fn random_integer(rng: &mut CrtRand, max: i64) -> i64 {
    rand_helper(rng, max)
}

/// UKismetMathLibrary::RandomIntegerInRange
pub fn random_integer_in_range(rng: &mut CrtRand, min: i64, max: i64) -> i64 {
    min + rand_helper(rng, max - min + 1)
}

/// UKismetArrayLibrary::Array_Shuffle
pub fn shuffle<T>(rng: &mut CrtRand, v: &mut [T]) {
    let last = v.len() as i64 - 1;
    let mut i = 0;
    while i <= last {
        let j = random_integer_in_range(rng, i, last);
        if i != j {
            v.swap(i as usize, j as usize);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges() {
        let mut r = CrtRand::new(1);
        for _ in 0..200 {
            let x = random_integer_in_range(&mut r, 3, 7);
            assert!((3..=7).contains(&x));
            assert!((0..4).contains(&random_integer(&mut r, 4)));
        }
        assert_eq!(random_integer(&mut r, 0), 0);
        let mut v: Vec<i32> = (0..10).collect();
        shuffle(&mut r, &mut v);
        let mut s = v.clone();
        s.sort();
        assert_eq!(s, (0..10).collect::<Vec<_>>());
        r.forced = [32767].into_iter().collect(); // FRand 1.0 -> RandHelper clamps to A - 1
        assert_eq!(random_integer(&mut r, 5), 4);
    }
}
