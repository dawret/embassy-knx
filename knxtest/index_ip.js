const { KNXTunnelSocket, DataPoints, KNXAddress } = require("knx-ip");

const knxClient = new KNXTunnelSocket("1.1.100");

knxClient.on(KNXTunnelSocket.KNXTunnelSocketEvents.error, err => {
    if (err) {
        console.log(err);
    }
});

const wait = (t = 3000) => {
    return new Promise(resolve => {
        setTimeout(() => { resolve(); }, t);
    });
};

const handleBusEvent = function (srcAddress, dstAddress, npdu) {
    console.log(`${srcAddress.toString()} -> ${dstAddress.toString()} :`, npdu.dataValue);
};

const testAddress = KNXAddress.createFromString("1.1.99", KNXAddress.TYPE_GROUP);
const testStatus = new DataPoints.Switch(
    KNXAddress.createFromString("1.1.98", KNXAddress.TYPE_GROUP)
);

testStatus.bind(knxClient);

const ip = "10.0.20.185"
const port = 3671
console.log("Connecting to ", ip, port);
knxClient.connectAsync(ip, port).then(() => console.log("Connected through channel id ", knxClient.channelID)).then(() => console.log("Reading test status"))
    .then(() => testStatus.read())
    .then(val => console.log("Test status:", val)).catch(err => { console.log(err); }).then(() => knxClient.disconnectAsync())
    .then(() => process.exit(0));
