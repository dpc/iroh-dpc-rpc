use super::*;

#[test]
fn it_works() {
    let _rpc = DpcRpc::builder(16u32)
        .handler(1, move |s, _w, _r| async move { println!("{s}") })
        .build();
}
