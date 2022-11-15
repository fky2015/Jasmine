
fn main() {
    
}

// #[tokio::main]
// async fn main() {
//     let voter_set: Vec<_> = (0..4).collect();
//     let genesis = data::Block::genesis();
//
//     let mut network = MemoryNetwork::new();
//
//     // Prepare the environment.
//     let nodes: Vec<_> = voter_set
//         .iter()
//         .map(|id| {
//             let adaptor = network.register(*id);
//             Node::new(
//                 *id,
//                 adaptor,
//                 genesis.to_owned(),
//                 VoterSet::new(voter_set.clone()),
//             )
//         })
//         .collect();
//
//     // Boot up the network.
//     let handle = tokio::spawn(async move {
//         network.dispatch().await;
//     });
//
//     let mut metrics = nodes.get(1).unwrap().metrics().await;
//
//     tokio::spawn(async move {
//         metrics.dispatch().await;
//     });
//
//     // Run the nodes.
//     nodes.into_iter().for_each(|mut node| {
//         tokio::spawn(async move {
//             node.run().await;
//         });
//     });
//
//     let _ = tokio::join!(handle);
// }
