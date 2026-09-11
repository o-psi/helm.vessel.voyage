package dev.helm.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.room.*
import dev.helm.vessel.*
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.serialization.json.*

@Entity(tableName = "mutations")
data class MutationRow(@PrimaryKey val commandId: String, val vesselId: String, val sessionId: String,
    val exactRequest: String, val state: String, val responseJson: String?)
@Entity(tableName = "cache", primaryKeys = ["vesselId", "key"])
data class CacheRow(val vesselId: String, val key: String, val json: String, val savedAt: Long)
@Dao
interface HelmDao {
    @Insert(onConflict = OnConflictStrategy.IGNORE) suspend fun insert(row: MutationRow): Long
    @Query("SELECT * FROM mutations WHERE commandId = :id") suspend fun mutation(id: String): MutationRow?
    @Query("UPDATE mutations SET state = :state, responseJson = :response WHERE commandId = :id AND (state != 'RESOLVED' OR :state = 'RESOLVED')")
    suspend fun record(id: String, state: String, response: String?)
    @Query("SELECT * FROM mutations WHERE vesselId = :vessel AND state = 'UNCERTAIN'") suspend fun pending(vessel: String): List<MutationRow>
    @Insert(onConflict = OnConflictStrategy.REPLACE) suspend fun cache(row: CacheRow)
    @Query("SELECT * FROM cache WHERE vesselId = :vessel AND `key` = :key") suspend fun cached(vessel: String, key: String): CacheRow?
}
@Database(entities = [MutationRow::class, CacheRow::class], version = 1, exportSchema = true)
abstract class HelmDatabase : RoomDatabase() {
    abstract fun dao(): HelmDao
    companion object {
        fun open(context: Context) = Room.databaseBuilder(context, HelmDatabase::class.java, "helm.db").build()
    }
}
/** Insert-if-absent is committed by Room before the transport may dispatch. Never replace an intent. */
class RoomJournal(private val database: HelmDatabase) : MutationJournal {
    override suspend fun insert(entry: JournalEntry): Boolean = database.withTransaction {
        val row = MutationRow(entry.commandId, entry.vesselId, entry.sessionId, entry.exactRequest, entry.state.name, entry.responseJson)
        if (database.dao().insert(row) != -1L) true else {
            val saved = requireNotNull(database.dao().mutation(entry.commandId))
            require(saved.vesselId == entry.vesselId && saved.sessionId == entry.sessionId && saved.exactRequest == entry.exactRequest) { "Command identity conflict" }
            false
        }
    }
    override suspend fun record(commandId: String, state: JournalState, responseJson: String?) = database.dao().record(commandId, state.name, responseJson)
    override suspend fun pending(vesselId: String) = database.dao().pending(vesselId).map {
        JournalEntry(it.vesselId, it.sessionId, it.commandId, it.exactRequest, JournalState.valueOf(it.state), it.responseJson)
    }
}
/** App-private, backup-disabled storage; AES-GCM key never leaves Android Keystore. No secret diagnostics. */
class CredentialStore(context: Context) {
    private val prefs = context.getSharedPreferences("connection", Context.MODE_PRIVATE)
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey("helm.connection.v1", null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder("helm.connection.v1", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    fun save(url: String, vessel: String, grant: String, token: String) {
        val plain = buildJsonObject { put("url", url); put("vessel", vessel); put("grant", grant); put("token", token) }.toString().toByteArray()
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
            val bytes = cipher.iv + cipher.doFinal(plain)
            check(prefs.edit().putString("encrypted", Base64.encodeToString(bytes, Base64.NO_WRAP)).commit()) { "Credential storage failed" }
        } finally { plain.fill(0) }
    }
    fun load(): JsonObject? {
        val encoded = prefs.getString("encrypted", null) ?: return null
        val bytes = Base64.decode(encoded, Base64.NO_WRAP)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12))) }
        val plain = cipher.doFinal(bytes.copyOfRange(12, bytes.size))
        return try { Json.parseToJsonElement(plain.toString(Charsets.UTF_8)).jsonObject } finally { plain.fill(0) }
    }
    fun forget() { check(prefs.edit().clear().commit()) }
}
