#[cfg(all(feature = "serialization", test))]
mod tests {
    extern crate base64;
    extern crate hdrsample;
    extern crate rand;

    use self::rand::Rng;

    use self::hdrsample::Histogram;
    use self::hdrsample::serialization::{Deserializer, Serializer, V2Serializer};
    use self::hdrsample::serialization::interval_log::{IntervalLogHeaderWriter,
                                                       IntervalLogHistogram, IntervalLogIterator,
                                                       LogEntry, LogIteratorError, Tag};

    use std::{io, str};
    use std::io::{BufRead, Read};
    use std::fs::File;
    use std::path::Path;

    #[test]
    fn parse_sample_tagged_interval_log_start_timestamp() {
        let data = load_iterator_from_file(Path::new("tests/data/tagged-Log.logV2.hlog"));
        let start_count = data.into_iter()
            .map(|r| r.unwrap())
            .filter_map(|e| match e {
                LogEntry::StartTime(t) => Some(t),
                _ => None,
            })
            .count();
        assert_eq!(1, start_count);
    }


    #[test]
    fn parse_sample_tagged_interval_log_interval_count() {
        let data = load_iterator_from_file(Path::new("tests/data/tagged-Log.logV2.hlog"));
        let intervals = data.into_iter()
            .map(|r| r.unwrap())
            .filter_map(|e| match e {
                LogEntry::Interval(ilh) => Some(ilh),
                _ => None,
            })
            .collect::<Vec<IntervalLogHistogram>>();

        assert_eq!(42, intervals.len());

        // half have tags, half do not
        assert_eq!(
            21,
            intervals.iter().filter(|ilh| ilh.tag().is_none()).count()
        );
        assert_eq!(
            21,
            intervals.iter().filter(|ilh| !ilh.tag().is_none()).count()
        );
    }

    #[test]
    fn parse_sample_tagged_interval_log_interval_metadata() {
        let data = load_iterator_from_file(Path::new("tests/data/tagged-Log.logV2.hlog"));
        let intervals = data.into_iter()
            .map(|r| r.unwrap())
            .filter_map(|e| match e {
                LogEntry::Interval(ilh) => Some(ilh),
                _ => None,
            })
            .collect::<Vec<IntervalLogHistogram>>();

        let expected = vec![
            (0.127, 1.007, 2.769),
            (1.134, 0.999, 0.442),
            (2.133, 1.001, 0.426),
            (3.134, 1.001, 0.426),
            (4.135, 0.997, 0.426),
            (5.132, 1.002, 0.426),
            (6.134, 0.999, 0.442),
            (7.133, 0.999, 0.459),
            (8.132, 1.0, 0.459),
            (9.132, 1.751, 1551.892),
            (10.883, 0.25, 0.426),
            (11.133, 1.003, 0.524),
            (12.136, 0.997, 0.459),
            (13.133, 0.998, 0.459),
            (14.131, 1.0, 0.492),
            (15.131, 1.001, 0.442),
            (16.132, 1.001, 0.524),
            (17.133, 0.998, 0.459),
            (18.131, 1.0, 0.459),
            (19.131, 1.0, 0.475),
            (20.131, 1.004, 0.475),
        ];

        // tagged and un-tagged are identical

        assert_eq!(
            expected,
            intervals
                .iter()
                .filter(|ilh| ilh.tag().is_none())
                .map(|ilh| { (ilh.start_timestamp(), ilh.duration(), ilh.max()) })
                .collect::<Vec<(f64, f64, f64)>>()
        );

        assert_eq!(
            expected,
            intervals
                .iter()
                .filter(|ilh| !ilh.tag().is_none())
                .map(|ilh| { (ilh.start_timestamp(), ilh.duration(), ilh.max()) })
                .collect::<Vec<(f64, f64, f64)>>()
        );

        let mut deserializer = Deserializer::new();
        for ilh in intervals {
            let serialized_histogram =
                base64::decode_config(ilh.encoded_histogram(), base64::STANDARD).unwrap();
            let decoded_hist: Histogram<u64> = deserializer
                .deserialize(&mut io::Cursor::new(&serialized_histogram))
                .unwrap();

            // this log happened to use 1000000 as the scaling factor. It was also formatted to 3
            // decimal places.
            assert_eq!(round(decoded_hist.max() as f64 / 1_000_000_f64), ilh.max());
        }
    }

    #[test]
    fn parse_sample_tagged_interval_log_rewrite_identical() {
        // trim off the comments and legend line
        let reader =
            io::BufReader::new(File::open(Path::new("tests/data/tagged-Log.logV2.hlog")).unwrap());

        // the sample log uses DEFLATE compressed histograms, which we can't match exactly, so the
        // best we can do is to re-serialize each one as uncompressed.

        let mut serializer = V2Serializer::new();
        let mut deserializer = Deserializer::new();

        let mut serialize_buf = Vec::new();
        let mut log_without_headers = Vec::new();
        reader
            .lines()
            .skip(4)
            .map(|r| r.unwrap())
            .for_each(|mut line| {
                let hist_index = line.rfind("HISTF").unwrap();
                let serialized =
                    base64::decode_config(&line[hist_index..], base64::STANDARD).unwrap();

                let decoded_hist: Histogram<u64> = deserializer
                    .deserialize(&mut io::Cursor::new(serialized))
                    .unwrap();

                serialize_buf.clear();
                serializer
                    .serialize(&decoded_hist, &mut serialize_buf)
                    .unwrap();

                // replace the deflate histogram with the predictable non-deflate one
                line.truncate(hist_index);
                line.push_str(&base64::encode_config(&serialize_buf, base64::STANDARD));

                log_without_headers.extend_from_slice(line.as_bytes());
                log_without_headers.extend_from_slice("\n".as_bytes());
            });

        let mut duplicate_log = Vec::new();

        {
            let mut writer =
                IntervalLogHeaderWriter::new(&mut duplicate_log, &mut serializer).into_log_writer();

            IntervalLogIterator::new(&log_without_headers)
                .map(|r| r.unwrap())
                .filter_map(|e| match e {
                    LogEntry::Interval(ilh) => Some(ilh),
                    _ => None,
                })
                .for_each(|ilh| {
                    let serialized_histogram =
                        base64::decode_config(ilh.encoded_histogram(), base64::STANDARD).unwrap();
                    let decoded_hist: Histogram<u64> = deserializer
                        .deserialize(&mut io::Cursor::new(&serialized_histogram))
                        .unwrap();

                    writer
                        .write_histogram(
                            &decoded_hist,
                            ilh.start_timestamp(),
                            ilh.duration(),
                            ilh.tag(),
                            1_000_000.0,
                        )
                        .unwrap();
                });
        }


        let orig_str = str::from_utf8(&log_without_headers).unwrap();
        let rewritten_str = str::from_utf8(&duplicate_log).unwrap();

        assert_eq!(orig_str, rewritten_str);
    }

    #[test]
    fn write_random_histograms_to_interval_log_then_read() {
        let mut rng = rand::thread_rng();

        let mut histograms = Vec::new();
        let mut tags = Vec::new();

        let mut log_buf = Vec::new();
        let mut serializer = V2Serializer::new();

        let max_scaling_factor = 1_000_000.0;

        {
            let mut writer =
                IntervalLogHeaderWriter::new(&mut log_buf, &mut serializer).into_log_writer();

            writer.write_comment("start").unwrap();

            for i in 0_u32..100 {
                let mut h = Histogram::<u64>::new_with_bounds(1, u64::max_value(), 3).unwrap();

                for _ in 0..100 {
                    // ensure no count above i64::max_value(), even when many large values are
                    // bucketed together
                    h.record_n(rng.gen::<u64>() >> 32, rng.gen::<u64>() >> 32)
                        .unwrap();
                }

                if rng.gen() {
                    tags.push(Some(format!("t{}", i)));
                } else {
                    tags.push(None);
                };
                let current_tag_str = tags.last().unwrap();
                let tag = current_tag_str
                    .as_ref()
                    .map(|s| Tag::new(s.as_str()).unwrap());

                writer
                    .write_histogram(&h, i as f64, (i as f64) + 10000.0, tag, max_scaling_factor)
                    .unwrap();

                writer.write_comment(&format!("line {}", i)).unwrap();

                histograms.push(h);
            }
        }

        let parsed = IntervalLogIterator::new(&log_buf)
            .map(|r| r.unwrap())
            .filter_map(|e| match e {
                LogEntry::Interval(ilh) => Some(ilh),
                _ => None,
            })
            .collect::<Vec<IntervalLogHistogram>>();

        assert_eq!(histograms.len(), parsed.len());

        let mut deserializer = Deserializer::new();
        for (index, ilh) in parsed.iter().enumerate() {
            let serialized_histogram =
                base64::decode_config(ilh.encoded_histogram(), base64::STANDARD).unwrap();
            let decoded_hist: Histogram<u64> = deserializer
                .deserialize(&mut io::Cursor::new(&serialized_histogram))
                .unwrap();
            let original_hist = &histograms[index];

            assert_eq!(original_hist, &decoded_hist);

            assert_eq!(index as f64, ilh.start_timestamp());
            assert_eq!((index as f64) + 10000.0, ilh.duration());
            assert_eq!(round(original_hist.max() as f64 / max_scaling_factor), ilh.max());
            let tag_string: Option<String> = tags.get(index).unwrap().as_ref().map(|s| s.clone());
            assert_eq!(tag_string, ilh.tag().map(|t| t.as_str().to_owned()));
        }
    }

    /// Round to 3 digits the way floats are in the log
    fn round(f: f64) -> f64 {
        format!("{:.3}", f).parse::<f64>().unwrap()
    }

    fn load_iterator_from_file<'a>(path: &Path) -> IntervalLogBufHolder {
        let mut buf = Vec::new();
        let _ = File::open(path).unwrap().read_to_end(&mut buf).unwrap();

        IntervalLogBufHolder { data: buf }
    }

    struct IntervalLogBufHolder {
        data: Vec<u8>,
    }

    impl<'a> IntoIterator for &'a IntervalLogBufHolder {
        type Item = Result<LogEntry<'a>, LogIteratorError>;
        type IntoIter = IntervalLogIterator<'a>;

        fn into_iter(self) -> Self::IntoIter {
            IntervalLogIterator::new(self.data.as_slice())
        }
    }
}