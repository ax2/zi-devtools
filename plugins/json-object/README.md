当前目录修订：JSON Object 0.1.1 candidate-03增加两项分类、别名及用途信息，保留原运行时/输出对象契约/权限；包内复核只读，10项实际WASI重放通过。用户在Studio选择的停用状态不受此提供方交付影响。

# ZiDevTools JSON Object plugin

Provider candidate `0.1.0` for the vendored Studio `result.json.v1@1.0.0-preview.1` profile. It exports a bounded JSON object summary and an object-valued receiver. Both capabilities declare whole-output schemas and opt in to `application/json` capture.

The provider preserves the `value` object as JSON through output and receiver input. Successful and failed responses use the complete `contractVersion`, `ok`, `data`, and `error` envelope required by the frozen WASI contract; only success responses have object-valued `data`, while failures keep `data: null`. Only the summary capability is registered as a Pi tool, behind the optional `pi.tools.register` grant. The receiver is not registered as an agent tool; the Host can invoke it only as the destination of a separately confirmed object transfer. The provider accepts one root object up to 8 KiB, 128 root fields, depth 64 and 4096 total nodes; the request and result envelopes are each bounded to 48 KiB. Duplicate keys, unsupported values, malformed requests and over-budget values fail without a successful `data` object.

This is an unsigned provider candidate. It is not Studio Host acceptance. The optional `pi.tools.register` permission is requested only when users choose to expose the summary tool to the current scene's AI; no file, storage, network or resource permission is requested. Install/profile negotiation, object capture and transfer through the real Host, confirmed save/readback, permission revocation, cancellation and failure isolation remain receiver-side checks.
