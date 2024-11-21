use std::{str::FromStr, time::Duration};

use boltz_client::{
    network::{electrum::ElectrumConfig, Chain},
    swaps::{
        boltzv2::{
            BoltzApiClientV2, CreateReverseRequest, CreateSubmarineRequest, Subscription,
            SwapUpdate, BOLTZ_TESTNET_URL_V2,
        },
        magic_routing::{check_for_mrh, sign_address},
    },
    util::{secrets::Preimage, setup_logger},
    Bolt11Invoice, LBtcSwapScriptV2, LBtcSwapTxV2, Secp256k1,
};

use bitcoin::{
    hashes::{sha256, Hash},
    hex::FromHex,
    key::rand::thread_rng,
    secp256k1::Keypair,
    Amount, PublicKey,
};

pub mod test_utils;

#[test]
#[ignore = "Requires testnet invoice and refund address"]
fn liquid_v2_submarine() {
    setup_logger();

    let secp = Secp256k1::new();
    let our_keys = Keypair::new(&secp, &mut thread_rng());

    let refund_public_key = PublicKey {
        inner: our_keys.public_key(),
        compressed: true,
    };

    // Set a new invoice string and refund address for each test.
    let invoice = "lntb1m1pnrv328pp5zymney8y48234em5lakrkuk8rfrftn5dkwfys7zghe2c40hxfmusdpz2djkuepqw3hjqnpdgf2yxgrpv3j8yetnwvcqz95xqyp2xqrzjqwyg6p2yhhqvq5d97kkwuk0mnrp3su6sn5fvtxn63gppms9fkegajzzxeyqq28qqqqqqqqqqqqqqq9gq2ysp5znw62my456pnzq7vyfgje2yjfat8gzgf88q8rl30dt3cgpmpk9eq9qyyssq55qds9y2vrtmqxq00fgrnartdhs0wwlt7u5uflzs5wnx8wad8y3y86y8lgre4qaszhvhesa6ts99g7m088j6dgjfe6hhtkfglqfqwjcp03v2nh".to_string();
    let refund_address = "tlq1qq0aa3lhat3r4auhstr0fsevj70gcwvvlsannf0ymlytelya2ylak7e69hksrk42fnl26wyk460ehy3pncxagy0ck47grlta62".to_string();

    let boltz_api_v2 = BoltzApiClientV2::new(BOLTZ_TESTNET_URL_V2);

    // If there is MRH send directly to that address
    // if let Some((bip21_addrs, amount)) =
    //     check_for_mrh(&boltz_api_v2, &invoice, Chain::BitcoinTestnet).unwrap()
    // {
    //     log::info!("Found MRH in invoice");
    //     log::info!("Send {} to {}", amount, bip21_addrs);
    //     return;
    // }

    // Initiate the swap with Boltz
    let create_swap_req = CreateSubmarineRequest {
        from: "L-BTC".to_string(),
        to: "BTC".to_string(),
        invoice: invoice.to_string(),
        refund_public_key,
        referral_id: None,
    };

    let create_swap_response = boltz_api_v2.post_swap_req(&create_swap_req).unwrap();

    log::info!("Got Swap Response from Boltz server");

    log::debug!("Swap Response: {:?}", create_swap_response);

    let swap_script =
        LBtcSwapScriptV2::submarine_from_swap_resp(&create_swap_response, refund_public_key)
            .unwrap();
    swap_script.to_address(Chain::LiquidTestnet).unwrap();

    log::debug!("Created Swap Script. : {:?}", swap_script);

    // Subscribe to websocket updates
    let mut socket = boltz_api_v2.connect_ws().unwrap();

    socket
        .send(tungstenite::Message::Text(
            serde_json::to_string(&Subscription::new(&create_swap_response.id)).unwrap(),
        ))
        .unwrap();

    // Event handlers for various swap status.
    loop {
        let response = serde_json::from_str(&socket.read().unwrap().to_string());

        if response.is_err() {
            if response.err().expect("expected").is_eof() {
                continue;
            }
        } else {
            match response.unwrap() {
                SwapUpdate::Subscription {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "subscribe");
                    assert!(channel == "swap.update");
                    assert!(args.get(0).expect("expected") == &create_swap_response.id);
                    log::info!(
                        "Successfully subscribed for Swap updates. Swap ID : {}",
                        create_swap_response.id
                    );
                }

                SwapUpdate::Update {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "update");
                    assert!(channel == "swap.update");
                    let update = args.get(0).expect("expected");
                    assert!(update.id == create_swap_response.id);
                    log::info!("Got Update from server: {}", update.status);

                    // Invoice is Set. Waiting for us to send onchain tx.
                    if update.status == "invoice.set" {
                        log::info!(
                            "Send {} sats to BTC address {}",
                            create_swap_response.expected_amount,
                            create_swap_response.address
                        );
                    }

                    // Boltz has paid the invoice, and waiting for our partial sig.
                    if update.status == "transaction.claim.pending" {
                        // Create the refund transaction at this stage
                        let swap_tx = LBtcSwapTxV2::new_refund(
                            swap_script.clone(),
                            &refund_address,
                            &ElectrumConfig::default_liquid(),
                        )
                        .unwrap();

                        let claim_tx_response = boltz_api_v2
                            .get_claim_tx_details(&create_swap_response.id)
                            .unwrap();

                        log::debug!("Received claim tx details : {:?}", claim_tx_response);

                        // Check that boltz have the correct preimage.
                        // At this stage the client should verify that LN invoice has been paid.
                        let preimage = Vec::from_hex(&claim_tx_response.preimage).unwrap();
                        let preimage_hash = sha256::Hash::hash(&preimage);
                        let invoice = Bolt11Invoice::from_str(&create_swap_req.invoice).unwrap();
                        let invoice_payment_hash = invoice.payment_hash();
                        assert!(invoice_payment_hash.to_string() == preimage_hash.to_string());
                        log::info!("Correct Hash preimage received from Boltz.");

                        // Compute and send Musig2 partial sig
                        let (partial_sig, pub_nonce) = swap_tx
                            .submarine_partial_sig(&our_keys, &claim_tx_response)
                            .unwrap();
                        boltz_api_v2
                            .post_claim_tx_details(&create_swap_response.id, pub_nonce, partial_sig)
                            .unwrap();
                        log::info!("Successfully Sent partial signature");
                    }

                    // This means the funding transaction was rejected by Boltz for whatever reason, and we need to get
                    // fund back via refund.
                    if update.status == "transaction.lockupFailed"
                        || update.status == "invoice.failedToPay"
                    {
                        let swap_tx = LBtcSwapTxV2::new_refund(
                            swap_script.clone(),
                            &refund_address,
                            &ElectrumConfig::default_liquid(),
                        )
                        .unwrap();

                        match swap_tx.sign_refund(
                            &our_keys,
                            Amount::from_sat(1000),
                            Some((&boltz_api_v2, &create_swap_response.id)),
                        ) {
                            Ok(tx) => {
                                let txid = swap_tx
                                    .broadcast(&tx, &ElectrumConfig::default_liquid(), None)
                                    .unwrap();
                                log::info!("Cooperative Refund Successfully broadcasted: {}", txid);
                            }
                            Err(e) => {
                                log::info!("Cooperative refund failed. {:?}", e);
                                log::info!("Attempting Non-cooperative refund.");

                                let tx = swap_tx
                                    .sign_refund(&our_keys, Amount::from_sat(1000), None)
                                    .unwrap();
                                let txid = swap_tx
                                    .broadcast(&tx, &ElectrumConfig::default_liquid(), None)
                                    .unwrap();
                                log::info!(
                                    "Non-cooperative Refund Successfully broadcasted: {}",
                                    txid
                                );
                            }
                        }
                    }

                    if update.status == "transaction.claimed" {
                        log::info!("Successfully completed submarine swap");
                        break;
                    }
                }

                SwapUpdate::Error {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "update");
                    assert!(channel == "swap.update");
                    let error = args.get(0).expect("expected");
                    log::error!(
                        "Got Boltz response error : {} for swap: {}",
                        error.error,
                        error.id
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "Requires testnet invoice and refund address"]
fn liquid_v2_reverse() {
    setup_logger();

    let secp = Secp256k1::new();
    let preimage = Preimage::new();
    let our_keys = Keypair::new(&secp, &mut thread_rng());
    let invoice_amount = 100000;
    let claim_public_key = PublicKey {
        compressed: true,
        inner: our_keys.public_key(),
    };

    // Give a valid claim address or else funds will be lost.
    let claim_address = "tlq1qqtzkefxathskcl5svkfwscd6eyhua8f8v9snpxdy7fe8lu3x6c0v93k3stc4e79avd4d9z76vm30yc3564z6wl5wcs2v409fl".to_string();

    let addrs_sig = sign_address(&claim_address, &our_keys).unwrap();

    let create_reverse_req = CreateReverseRequest {
        invoice_amount,
        from: "BTC".to_string(),
        to: "L-BTC".to_string(),
        preimage_hash: preimage.sha256,
        address_signature: Some(addrs_sig.to_string()),
        address: Some(claim_address.clone()),
        claim_public_key,
        referral_id: None,
    };

    let boltz_api_v2 = BoltzApiClientV2::new(BOLTZ_TESTNET_URL_V2);

    let reverse_resp = boltz_api_v2.post_reverse_req(create_reverse_req).unwrap();

    let _ = check_for_mrh(&boltz_api_v2, &reverse_resp.invoice, Chain::BitcoinTestnet).unwrap().unwrap();

    log::debug!("Got Reverse swap response: {:?}", reverse_resp);

    let swap_script =
        LBtcSwapScriptV2::reverse_from_swap_resp(&reverse_resp, claim_public_key).unwrap();
    swap_script.to_address(Chain::LiquidTestnet).unwrap();

    // Subscribe to wss status updates
    let mut socket = boltz_api_v2.connect_ws().unwrap();

    let subscription = Subscription::new(&reverse_resp.id);

    socket
        .send(tungstenite::Message::Text(
            serde_json::to_string(&subscription).unwrap(),
        ))
        .unwrap();

    // Event handlers for various swap status.
    loop {
        let response = serde_json::from_str(&socket.read().unwrap().to_string());

        if response.is_err() {
            if response.err().expect("expected").is_eof() {
                continue;
            }
        } else {
            match response.as_ref().unwrap() {
                SwapUpdate::Subscription {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "subscribe");
                    assert!(channel == "swap.update");
                    assert!(args.get(0).expect("expected") == &reverse_resp.id);
                    log::info!("Subscription successful for swap : {}", &reverse_resp.id);
                }

                SwapUpdate::Update {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "update");
                    assert!(channel == "swap.update");
                    let update = args.get(0).expect("expected");
                    assert!(&update.id == &reverse_resp.id);
                    log::info!("Got Update from server: {}", update.status);

                    if update.status == "swap.created" {
                        log::info!("Waiting for Invoice to be paid: {}", &reverse_resp.invoice);
                        continue;
                    }

                    if update.status == "transaction.mempool" {
                        log::info!("Boltz broadcasted funding tx");

                        std::thread::sleep(Duration::from_secs(15));

                        let claim_tx = LBtcSwapTxV2::new_claim(
                            swap_script.clone(),
                            claim_address.clone(),
                            &ElectrumConfig::default_liquid(),
                        )
                        .unwrap();

                        let tx = claim_tx
                            .sign_claim(
                                &our_keys,
                                &preimage,
                                Amount::from_sat(1000),
                                Some((&boltz_api_v2, reverse_resp.id.clone())),
                            )
                            .unwrap();

                        claim_tx
                            .broadcast(&tx, &ElectrumConfig::default_liquid(), None)
                            .unwrap();

                        // To test Lowball broadcast uncomment below line
                        // claim_tx
                        //     .broadcast(
                        //         &tx,
                        //         &ElectrumConfig::default_liquid(),
                        //         Some((&boltz_api_v2, boltz_client::network::Chain::LiquidTestnet)),
                        //     )
                        //     .unwrap();

                        log::info!("Succesfully broadcasted claim tx!");
                        log::debug!("Claim Tx {:?}", tx);
                    }

                    if update.status == "invoice.settled" {
                        log::info!("Reverse Swap Successful!");
                        break;
                    }
                }

                SwapUpdate::Error {
                    event,
                    channel,
                    args,
                } => {
                    assert!(event == "update");
                    assert!(channel == "swap.update");
                    let error = args.get(0).expect("expected");
                    println!("Got error : {} for swap: {}", error.error, error.id);
                }
            }
        }
    }
}