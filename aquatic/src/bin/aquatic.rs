use aquatic;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    aquatic::run();
}