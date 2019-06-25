use crate::models::Money;
use askama::Error;
use chrono::{Duration, NaiveDateTime};
use chrono_humanize::{Accuracy, HumanTime, Tense};
pub fn grin(nanogrins: &i64) -> Result<String, Error> {
    Ok(Money::from_grin(*nanogrins).to_string())
}

pub fn pretty_date(date: &NaiveDateTime) -> Result<String, Error> {
    Ok(date.format("%d.%m.%Y %H:%M:%S").to_string())
}

pub trait ForHuman {
    fn for_human(&self) -> String;
}

impl ForHuman for Duration {
    fn for_human(&self) -> String {
        let ht = HumanTime::from(*self);
        if self.num_milliseconds() > 0 {
            ht.to_text_en(Accuracy::Precise, Tense::Future)
        } else {
            ht.to_text_en(Accuracy::Precise, Tense::Past)
        }
    }
}

pub fn duration(duration: &Duration) -> Result<String, Error> {
    let ht = HumanTime::from(*duration);
    if duration.num_milliseconds() > 0 {
        Ok(ht.to_text_en(Accuracy::Precise, Tense::Future))
    } else {
        Ok(ht.to_text_en(Accuracy::Precise, Tense::Past))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_test() {
        let d = Duration::seconds(132);
        let res = duration(&d).unwrap();
        println!("\x1B[31;1m res\x1B[0m = {:?}", res);
        assert!(res == s!("in 2 minutes and 12 seconds"));

        let d2 = Duration::seconds(-132);
        let res2 = duration(&d2).unwrap();
        println!("\x1B[31;1m res2\x1B[0m = {:?}", res2);
        assert!(res2 == s!("2 minutes and 12 seconds ago"));
    }
    #[test]
    fn human_time_test() {
        let d = Duration::seconds(132);
        let ht = HumanTime::from(d);
        println!(
            "Present {}",
            ht.to_text_en(Accuracy::Precise, Tense::Present)
        );
        println!("Future {}", ht.to_text_en(Accuracy::Precise, Tense::Future));
        println!("Past {}", ht.to_text_en(Accuracy::Precise, Tense::Past));
    }
}
