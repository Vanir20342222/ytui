package dev.ytui.android;

import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** OS credential storage adapter; key material never leaves Android Keystore. */
final class AccountCipher {
  private static final String ALIAS = "ytui.account.v1";
  private static final String PREFIX = "ytui-account-v1:";
  private static final byte[] AAD = "dev.ytui.android/account/v1".getBytes(StandardCharsets.UTF_8);
  private static final int LIMIT = 2 * 1024 * 1024;

  private static synchronized SecretKey key(boolean create) throws Exception {
    KeyStore store = KeyStore.getInstance("AndroidKeyStore");
    store.load(null);
    if (store.containsAlias(ALIAS)) return (SecretKey) store.getKey(ALIAS, null);
    if (!create) throw new GeneralSecurityException("Account key is unavailable");
    KeyGenerator generator =
        KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
    generator.init(
        new KeyGenParameterSpec.Builder(
                ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            .build());
    return generator.generateKey();
  }

  static String seal(String text) throws Exception {
    byte[] plain = text.getBytes(StandardCharsets.UTF_8);
    if (plain.length > LIMIT) throw new GeneralSecurityException("Account exceeds size limit");
    Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
    cipher.init(Cipher.ENCRYPT_MODE, key(true));
    cipher.updateAAD(AAD);
    byte[] iv = cipher.getIV(), encrypted = cipher.doFinal(plain);
    if (iv.length != 12) throw new GeneralSecurityException("Invalid account nonce length");
    byte[] envelope =
        ByteBuffer.allocate(iv.length + encrypted.length).put(iv).put(encrypted).array();
    return PREFIX + Base64.encodeToString(envelope, Base64.NO_WRAP);
  }

  static String open(String envelope) throws Exception {
    if (!envelope.startsWith(PREFIX) || envelope.length() > 3 * 1024 * 1024)
      throw new GeneralSecurityException("Invalid account envelope");
    byte[] bytes = Base64.decode(envelope.substring(PREFIX.length()), Base64.NO_WRAP);
    if (bytes.length < 28 || bytes.length > LIMIT + 28)
      throw new GeneralSecurityException("Invalid account payload length");
    Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
    cipher.init(Cipher.DECRYPT_MODE, key(false), new GCMParameterSpec(128, bytes, 0, 12));
    cipher.updateAAD(AAD);
    byte[] plain = cipher.doFinal(bytes, 12, bytes.length - 12);
    return StandardCharsets.UTF_8.newDecoder().decode(ByteBuffer.wrap(plain)).toString();
  }

  private AccountCipher() {}
}
