use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, ManageConnection, Pool, PooledConnection};
use diesel_migrations::{
    find_migrations_directory, run_pending_migrations_in_directory, MigrationConnection,
};
use std::env;
use std::path::{Path, PathBuf};

pub fn run_migrations<Conn>(conn: &Conn)
where
    Conn: MigrationConnection,
{
    use std::io::Cursor;
    use std::str;
    let mut buf = Cursor::new(vec![0; 100]);

    let res =
        run_pending_migrations_in_directory(conn, &find_migrations_directory().unwrap(), &mut buf);
    println!("{}", str::from_utf8(&buf.into_inner()).unwrap());
    res.unwrap();
}

pub fn get_test_pool() -> Pool<ConnectionManager<PgConnection>> {
    let database_url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");
    pool
}
