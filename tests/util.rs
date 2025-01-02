use geojson::{Feature, Value};
use supercluster::DataRange;

pub fn get_data_range(data: &Vec<Feature>) -> Option<DataRange> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for feature in data {
        if let Some(geometry) = &feature.geometry {
            if let Value::Point(ref coords) = geometry.value {
                let x = coords[0];
                let y = coords[1];
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some(DataRange {
            min_x,
            max_x,
            min_y,
            max_y,
        })
    } else {
        None
    }
}
