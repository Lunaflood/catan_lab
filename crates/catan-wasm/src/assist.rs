//! An optional adviser for the worker's replay. It never plays a move or shares
//! the live opponents' RNG/beliefs. Reasons use the viewer's observation only.
use super::*;
use catan_ai::agent_v2::{AgentV2, V2Config};
use catan_core::observation::{Observation, LEGACY_APP};

#[no_mangle]
pub extern "C" fn assist_start(seat: u32) -> u32 {
    SESSION.with(|s| {
        let mut b=s.borrow_mut(); let Some(sess)=b.as_mut() else { return 0; };
        if seat as usize >= sess.game.n() || !sess.humans[seat as usize] { return 0; }
        sess.advisor=Some((seat as u8,AgentV2::new(0xA551_5700 + seat as u64,V2Config::v4())));
        1
    })
}

#[no_mangle]
pub extern "C" fn assist_json() -> *const u8 {
    SESSION.with(|s| {
        let mut b=s.borrow_mut(); let Some(sess)=b.as_mut() else { return put_out("null".into()); };
        let Some((seat,advisor))=sess.advisor.as_mut() else { return put_out("null".into()); };
        if sess.game.is_over() || sess.game.to_act != *seat || sess.actions.is_empty() { return put_out("null".into()); }
        let obs=catan_core::observer::observe(&sess.game,*seat,&sess.actions,LEGACY_APP,sess.evseq);
        let chosen=advisor.decide(&obs);
        let Some(index)=sess.actions.iter().position(|a| *a==chosen) else { return put_out("null".into()); };
        let mut j=Json::new(); j.obj_start();
        j.key("index"); j.num(index as f32);
        j.key("reasons"); j.arr_start();
        for reason in reasons(&obs,chosen) { j.str(&reason); }
        j.arr_end();
        j.key("alternatives"); j.arr_start();
        for v in advisor.last.top.iter().filter(|v| v.action != chosen).take(2) {
            if let Some(i)=sess.actions.iter().position(|a| *a==v.action) { j.num(i as f32); }
        }
        j.arr_end(); j.obj_end(); put_out(j.finish())
    })
}

fn reasons(obs: &Observation, a: Action) -> Vec<String> {
    let g=&obs.public.game; let me=obs.viewer;
    let mut out=Vec::new();
    match a {
        Action::SetupSettlement(n) | Action::BuildSettlement(n) | Action::BuildCity(n) => {
            let city=matches!(a,Action::BuildCity(_));
            let topo=Topology::get();
            let production: Vec<String>=topo.node_tiles[n as usize].into_iter().filter_map(|t| {
                g.board.tile_resource[t as usize].map(|r| format!("{}の{}",g.board.tile_number[t as usize],r.ja()))
            }).collect();
            if !production.is_empty() {
                out.push(format!("{}に面する場所です。",production.join("・")));
                let pips: u16 = g.board.node_pips(n).iter().map(|&v| v as u16).sum();
                out.push(format!("盗賊がいない場合、{}平均産出はサイコロ1回あたり約{:.2}枚です。", if city { "都市化による追加の" } else { "この場所の" }, pips as f32 / 36.0));
            }
            out.push(if city { "勝利点が1点増え、この場所の資源産出が2倍になります。" }
                else if matches!(a,Action::SetupSettlement(_)) { "出やすい数字、資源の組み合わせ、次の配置と相手との場所の取り合いを評価しています。" }
                else { "勝利点が1点増え、資源を得られる拠点が増えます。" }.into());
            if let Some(port)=g.board.node_port[n as usize] {
                out.push(match port { PortKind::Generic => "3:1の港を利用できます。".into(), PortKind::Specific(r)=>format!("{}を2:1で交換できる港を利用できます。",r.ja()) });
            }
            if !matches!(a,Action::SetupSettlement(_)) && obs.own.actual_vp >= 9 { out.push("この建設で10点に届き、勝利できます。".into()); }
        }
        Action::SetupRoad(e) | Action::BuildRoad(e) => {
            out.push("次の開拓地への到達、相手による進路の遮断、最長交易路への効果を比較して選んだ道です。".into());
            let before = catan_core::longest_road::longest_road_length(&g.board,me);
            let mut after = g.clone(); after.board.road[e as usize]=Some(me);
            let length = catan_core::longest_road::longest_road_length(&after.board,me);
            if length > before { out.push(format!("自分の最長の道は{}本から{}本になります。",before,length)); }
            let new_sites=(0..NUM_NODES).filter(|&n| !g.can_place_settlement(n as u8,me,false) && after.can_place_settlement(n as u8,me,false)).count();
            if new_sites>0 { out.push(format!("開拓地を建てられる場所が新たに{}か所増えます（資源と残り駒は別途必要です）。",new_sites)); }
        },
        Action::BuyDevCard => out.push("都市などの建設と比較し、騎士・勝利点・資源獲得カードの可能性を含めて購入を評価しています。引く種類は未確定です。".into()),
        Action::MoveRobber { tile, victim } => {
            let number=g.board.tile_number[tile as usize];
            let mut loss=[0u16;MAX_PLAYERS];
            for node in Topology::get().tile_nodes[tile as usize] {
                if let Some(b)=g.board.building[node as usize] { loss[b.owner as usize] += if b.kind==BuildingKind::City { 2 } else { 1 }; }
            }
            if g.board.tile_resource[tile as usize].is_some() {
                let units:u16=loss.iter().enumerate().filter(|(p,_)| *p!=me as usize).map(|(_,n)| *n).sum();
                if units>0 { out.push(format!("{}が出たとき、このマスから相手に渡る計{}枚の産出を止められます。",number,units)); }
                if loss[me as usize]>0 { out.push(format!("自分の産出も{}枚分止まる点には注意してください。",loss[me as usize])); }
            }
            out.push(if victim.is_some() { "表示した相手から資源を1枚奪います。相手の勝利への近さも比較しています。" } else { "ここには資源を奪える相手がいません。盗賊の移動だけを行います。" }.into());
        }
        Action::OfferTrade {..} | Action::CounterOffer {..} | Action::AcceptTrade | Action::ConfirmTrade(_) | Action::AcceptCounter {..} | Action::CounterAlt {..} => out.push("交換後にできる建設と、相手が得る利益・勝利への近さを合わせて評価しています。提案した交換が成立するとは限りません。".into()),
        Action::RejectTrade | Action::CancelTrade | Action::CounterAltRemove(_) => out.push("現在の交換条件を進めるより、資源を残す選択を高く評価しています。相手に渡る利益も考慮しています。".into()),
        Action::MaritimeTrade {..} => out.push("手元の余剰資源を不足資源に換え、この後の建設や発展カード購入につなげる候補です。".into()),
        Action::Discard(_) => out.push("捨てた後にできる建設・交換を比較し、残す資源の組み合わせを選んでいます。".into()),
        Action::Roll => out.push("サイコロで資源を確定させてから、建設・交換の判断へ進みます。次の出目は予測に使っていません。".into()),
        Action::EndTurn => out.push("今使える資源・発展カードを残す選択を、現在の建設・交換候補より高く評価しています。".into()),
        Action::PlayKnight => out.push("盗賊を動かす効果と、使用済み騎士の増加による最大騎士力への効果を評価しています。".into()),
        Action::PlayRoadBuilding => out.push("資源を払わず道を最大2本建てられます。拡張先と最長交易路への効果を評価しています。".into()),
        Action::PlayYearOfPlenty(..) | Action::PlayMonopoly(_) => out.push("資源獲得後の建設・交換を比較して、使う資源を選んでいます。".into()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advice_is_legal_and_does_not_play_or_reveal_hidden_state() {
        game_new(42,2,3,4,4,1,0,1);
        assert_eq!(assist_start(4),0); assert_eq!(assist_start(1),0); assert_eq!(assist_start(0),1);
        let before=fingerprint();
        assist_json();
        let text=OUT.with(|o| String::from_utf8(o.borrow().clone()).unwrap());
        assert!(text.starts_with("{\"index\":")); assert!(text.contains("reasons"));
        assert_eq!(fingerprint(),before);
        SESSION.with(|s| { let b=s.borrow(); let sess=b.as_ref().unwrap(); assert_eq!(sess.seq,0); assert_eq!(sess.evseq,0); assert!(sess.log.is_empty()); });
        assert_eq!(apply_index(0),1); assert_eq!(apply_index(0),1);
        assist_json();
        assert_eq!(OUT.with(|o| o.borrow().clone()),b"null"); // Now the other seat acts.
    }
}
