const {KNXClient, KNXClientEvents} = require("knxultimate");

const options = {
    ipAddr: "10.0.20.185",
    ipPort: "3671",
    physAddr: "1.1.100",
    suppress_ack_ldatareq: false,
    loglevel: "info",
    hostProtocol: "TunnelUDP",
    isSecureKNXEnabled: false,
    jKNXSecureKeyring: "",
    localIPAddress: "",
    KNXQueueSendIntervalMilliseconds:25
};

const client = new KNXClient(options);
//console.log(client);

client.on(KNXClientEvents.connected, info => {
    console.log("Connected: ", info);
});
client.on(KNXClientEvents.error, err => {
    // Error event
    console.log("Error", err)
});
client.on(KNXClientEvents.connecting, info => {
    // The client is setting up the connection
    console.log("Connecting...", info)
});
client.Connect();

setTimeout(() => {
    client.Disconnect();
    process.exit
}, 2000);