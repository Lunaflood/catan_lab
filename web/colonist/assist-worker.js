// Reconstruct a separate game. Only the adviser receives projected observations.
self.onmessage = async ({data}) => {
  try {
    const {setup:s, journal, fingerprint, requestId} = data;
    const response = await fetch("catan_wasm.wasm", {cache:"no-store"});
    if (!response.ok) throw Error("アシストの読み込みに失敗しました。");
    const {instance} = await WebAssembly.instantiate(await response.arrayBuffer(), {});
    const w = instance.exports;
    w.game_new(s.boardSeed,s.diceSeed,s.devSeed,s.stealSeed,s.players,1<<s.mySeat,0,s.askLast);
    if (!w.assist_start(s.mySeat)) throw Error("この席のアシストを開始できません。");
    const custom={offer:"offer_custom",counter:"counter_custom",alt:"counter_alt",altRemove:"counter_alt_remove",finish:"counter_finish",discard:"discard_custom",maritime:"maritime_bulk"};
    for (const move of journal) {
      const ok = move.i!==undefined ? w.apply_index(move.i) : w[custom[move.k]]?.(...(move.a||[]));
      if (ok!==1) throw Error("対局の記録が一致しません。画面を更新してから試してください。");
    }
    if ((w.fingerprint()>>>0)!==fingerprint) throw Error("現在の盤面を再現できませんでした。");
    const ptr=w.assist_json();
    const advice=JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,ptr,w.out_len())));
    if (!advice) throw Error("自分が操作できるときに使ってください。");
    self.postMessage({requestId,fingerprint,advice});
  } catch (error) { self.postMessage({requestId:data.requestId,error:error.message}); }
};
