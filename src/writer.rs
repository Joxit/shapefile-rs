//! Module with the definition of the [Writer](struct.Writer.html) that allows writing shapefile
//!
//! It is recommended to create a `Writer` using its [from_path](struct.Writer.html#method.from_path) method
//! to ensure that both the .shp and .shx files are created.
//! Then use its [writes_shapes](struct.Writer.html#method.write_shapes) method to write the files.

use std::io::{BufWriter, Write};

use header;
use record::{EsriShape, RecordHeader};
use std::fs::File;
use std::path::Path;
use Error;

use byteorder::{BigEndian, WriteBytesExt};
use reader::ShapeIndex;

fn f64_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

fn f64_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

fn write_index_file<T: Write>(
    mut dest: &mut T,
    shapefile_header: &header::Header,
    shapes_index: Vec<ShapeIndex>,
) -> Result<(), std::io::Error> {
    let mut header = *shapefile_header;
    let content_len = shapes_index.len() * 2 * std::mem::size_of::<i32>();
    header.file_length = header::HEADER_SIZE + content_len as i32;
    header.file_length /= 2;

    header.write_to(&mut dest)?;
    for shape_index in shapes_index {
        dest.write_i32::<BigEndian>(shape_index.offset)?;
        dest.write_i32::<BigEndian>(shape_index.record_size)?;
    }
    Ok(())
}

/// struct that writes the shapes
pub struct Writer<T: Write> {
    pub dest: T,
    index_dest: Option<T>,
    dbase_dest: Option<T>,
}

impl<T: Write> Writer<T> {
    /// Creates a writer that can be used to write a new shapefile.
    ///
    /// The `dest` argument is only for the .shp
    pub fn new(dest: T) -> Self {
        Self {
            dest,
            index_dest: None,
            dbase_dest: None,
        }
    }

    //TODO This method should move as calling it twice would produce a shitty file
    /// Writes the shapes to the file
    ///
    /// # Examples
    ///
    /// ```
    /// use shapefile::Point;
    /// let mut writer = shapefile::Writer::from_path("points.shp").unwrap();
    /// let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)];
    ///
    /// writer.write_shapes(points).unwrap();
    /// ```
    ///
    /// ```
    /// use shapefile::{Point, Polyline};
    /// let mut writer = shapefile::Writer::from_path("polylines.shp").unwrap();
    /// let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)];
    /// let polyline = Polyline::new(points);
    ///
    /// writer.write_shapes(vec![polyline]).unwrap();
    /// ```
    pub fn write_shapes<S: EsriShape>(&mut self, shapes: Vec<S>) -> Result<(), Error> {
        let mut file_length = header::HEADER_SIZE as usize;
        for shape in &shapes {
            file_length += 2 * std::mem::size_of::<i32>(); // record_header
            file_length += std::mem::size_of::<i32>(); // shape_type
            file_length += shape.size_in_bytes();
        }
        file_length /= 2; // file size is in 16bit words

        if file_length > i32::max_value() as usize {
            panic!("To big"); //TODO convert in proper error
        }

        let mut point_min = [std::f64::MAX, std::f64::MAX, std::f64::MAX];
        let mut point_max = [std::f64::MIN, std::f64::MIN, std::f64::MIN];
        let mut m_range = [0.0, 0.0];
        for shape in &shapes {
            let bbox = shape.bbox();
            let z_range = shape.z_range();
            point_min[0] = f64_min(point_min[0], bbox.xmin);
            point_min[1] = f64_min(point_min[1], bbox.ymin);
            point_min[2] = f64_min(point_min[2], z_range[0]);

            point_max[0] = f64_max(point_max[0], bbox.xmax);
            point_max[1] = f64_max(point_max[1], bbox.ymax);
            point_max[2] = f64_max(point_max[2], z_range[1]);

            let s_m_range = shape.m_range();
            m_range[0] = f64_min(m_range[0], s_m_range[0]);
            m_range[1] = f64_max(m_range[1], s_m_range[1]);
        }

        if point_min[2] == std::f64::MAX {
            point_min[2] = 0.0;
        }

        if point_max[2] == std::f64::MIN {
            point_max[2] = 0.0;
        }

        let file_length = file_length as i32;
        let shapetype = S::shapetype();
        let header = header::Header {
            file_length,
            point_min,
            point_max,
            m_range,
            shape_type: shapetype,
            version: 1000,
        };

        let mut pos = header::HEADER_SIZE;
        header.write_to(&mut self.dest)?;
        let mut shapes_index = Vec::<ShapeIndex>::with_capacity(shapes.len());
        for (i, shape) in shapes.into_iter().enumerate() {
            //TODO Check record size < i32_max ?
            let record_size = (shape.size_in_bytes() + std::mem::size_of::<i32>()) / 2;
            let rc_hdr = RecordHeader {
                record_number: i as i32,
                record_size: record_size as i32,
            };

            shapes_index.push(ShapeIndex {
                offset: pos,
                record_size: record_size as i32,
            });

            rc_hdr.write_to(&mut self.dest)?;
            shapetype.write_to(&mut self.dest)?;
            shape.write_to(&mut self.dest)?;
            pos += (record_size * 2) as i32;
        }

        if let Some(ref mut shx_dest) = &mut self.index_dest {
            write_index_file(shx_dest, &header, shapes_index)?;
        }

        Ok(())
    }

    pub fn write_shapes_and_records<S: EsriShape>(
        mut self,
        shapes: Vec<S>,
        records: Vec<dbase::Record>,
    ) -> Result<(), Error> {
        if shapes.len() != records.len() {
            panic!("The shapes and records vectors must have the same len");
        }
        self.write_shapes(shapes)?;
        if let Some(dbase_dest) = self.dbase_dest {
            let dbase_writer = dbase::Writer::new(dbase_dest);
            dbase_writer.write(&records)?;
        }
        Ok(())
    }

    /// Adds dest as the destination where the index file will be written
    pub fn add_index_dest(&mut self, dest: T) {
        self.index_dest = Some(dest);
    }

    /// Adds dest as the destination where the dbase content will be written
    pub fn add_dbase_dest(&mut self, dest: T) {
        self.dbase_dest = Some(dest);
    }
}

impl Writer<BufWriter<File>> {
    /// Creates a new writer from a path.
    /// Creates both a .shp and .shx files
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// let writer = shapefile::Writer::from_path("/dev/null");
    /// ```
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let shp_path = path.as_ref().to_path_buf();
        let shx_path = shp_path.with_extension("shx");
        let dbf_path = shp_path.with_extension("dbf");

        let shp_file = BufWriter::new(File::create(shp_path)?);
        let shx_file = BufWriter::new(File::create(shx_path)?);
        let dbf_file = BufWriter::new(File::create(dbf_path)?);

        let mut writer = Self::new(shp_file);
        writer.add_index_dest(shx_file);
        writer.add_dbase_dest(dbf_file);

        Ok(writer)
    }
}
